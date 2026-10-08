import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { NemligSuggestions } from "@/components/NemligSuggestions";
import type { NemligProduct } from "@/lib/api";

const MILK: NemligProduct = {
  id: "701012",
  name: "Letmælk 1,5% øko.",
  description: "1 l / Arla ØKO",
  price: 13.95,
  unit_price: "13,95 kr/l",
  image: null,
  available: true,
  offer: "12,95 kr",
};

describe("NemligSuggestions", () => {
  it("searches once typing pauses, and offers what it found", async () => {
    const search = vi.fn().mockResolvedValue([MILK]);
    const onChoose = vi.fn();
    render(<NemligSuggestions query="letmælk" search={search} onChoose={onChoose} />);
    fireEvent.click(await screen.findByRole("button", { name: /Letmælk 1,5% øko/ }, { timeout: 2000 }));
    expect(search).toHaveBeenCalledTimes(1);
    expect(onChoose).toHaveBeenCalledWith(MILK);
    expect(screen.getByText("On offer")).toBeInTheDocument();
  });

  it("asks nothing for a single letter", async () => {
    const search = vi.fn().mockResolvedValue([MILK]);
    render(<NemligSuggestions query="l" search={search} onChoose={vi.fn()} />);
    await new Promise((r) => setTimeout(r, 500));
    expect(search).not.toHaveBeenCalled();
  });
});
