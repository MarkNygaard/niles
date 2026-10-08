import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { NemligOrder } from "@/components/NemligOrder";
import type { NemligOrderProps } from "@/components/NemligOrder";
import type { DeliveryDay, NemligSent } from "@/lib/api";

const SENT: NemligSent = {
  basket: {
    lines: [
      { product_id: "701012", name: "Letmælk 1,5% øko.", quantity: 2, total: 27.9 },
      { product_id: "5060220", name: "Surdejsrundstykker", quantity: 1, total: 14.95 },
    ],
    total: 101.85,
    delivery_price: 59,
    delivery: null,
    slot_id: null,
    minimum_total: 500,
    meets_minimum: false,
  },
  sent: 2,
  without: ["Skyr"],
  checkout: "https://www.nemlig.com/basket",
};

const DAYS: DeliveryDay[] = [
  {
    date: "2026-10-09",
    slots: [
      { id: 1, start_hour: 7, end_hour: 9, price: 46, available: true, selected: false, deadline: null },
      { id: 2, start_hour: 17, end_hour: 19, price: 39, available: false, selected: false, deadline: null },
    ],
  },
  {
    date: "2026-10-10",
    slots: [{ id: 3, start_hour: 7, end_hour: 15, price: 21, available: true, selected: false, deadline: null }],
  },
];

function open(props: Partial<NemligOrderProps> = {}) {
  const onReserve = vi.fn();
  render(<NemligOrder sent={SENT} days={DAYS} onReserve={onReserve} onClose={vi.fn()} {...props} />);
  return { onReserve };
}

describe("NemligOrder", () => {
  it("says what went in and what did not", () => {
    open();
    expect(screen.getByText(/2 items sent · 2 in the basket · 101,85 kr/)).toBeInTheDocument();
    expect(screen.getByText(/no product chosen: Skyr/)).toBeInTheDocument();
  });

  it("warns below nemlig.com's smallest order", () => {
    open();
    expect(screen.getByText(/smallest order is 500,00 kr/)).toBeInTheDocument();
  });

  it("reserves the time pressed", () => {
    const { onReserve } = open();
    fireEvent.click(screen.getByRole("button", { name: /07–09/ }));
    expect(onReserve).toHaveBeenCalledWith(1);
  });

  it("does not offer a time that is gone", () => {
    open();
    expect(screen.getByRole("button", { name: /17–19/ })).toBeDisabled();
  });

  it("shows another day's times", () => {
    const { onReserve } = open();
    fireEvent.click(screen.getAllByRole("tab")[1]);
    fireEvent.click(screen.getByRole("button", { name: /07–15/ }));
    expect(onReserve).toHaveBeenCalledWith(3);
  });

  it("says when a time is reserved", () => {
    open({ sent: { ...SENT, basket: { ...SENT.basket, delivery: "Fredag 10. oktober kl. 7-15", slot_id: 3 } } });
    expect(screen.getByText("Fredag 10. oktober kl. 7-15")).toBeInTheDocument();
  });

  it("says how long a reserved time is held", () => {
    open({
      sent: { ...SENT, basket: { ...SENT.basket, delivery: "Fredag 10. oktober kl. 7-15", slot_id: 3 } },
      heldUntil: new Date(2026, 9, 8, 19, 42),
    });
    expect(screen.getByText(/Held until .*19.42/)).toBeInTheDocument();
  });

  it("sends payment to nemlig.com rather than doing it here", () => {
    open();
    const link = screen.getByRole("link", { name: /Finish at nemlig.com/ });
    expect(link).toHaveAttribute("href", "https://www.nemlig.com/basket");
    expect(link).toHaveAttribute("target", "_blank");
  });
});
