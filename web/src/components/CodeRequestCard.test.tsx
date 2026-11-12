import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { CodeRequestCard } from "@/components/CodeRequestCard";
import type { CodeRequest } from "@/lib/api";

const REQUEST: CodeRequest = {
  id: "c1",
  label: "Shop",
  site: "https://shop.example",
  status: "waiting",
};

function renderCard(props: Partial<Parameters<typeof CodeRequestCard>[0]> = {}) {
  const onEnter = vi.fn();
  render(<CodeRequestCard request={REQUEST} onEnter={onEnter} {...props} />);
  return onEnter;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("CodeRequestCard", () => {
  it("names the site that sent the code", () => {
    renderCard();
    const card = screen.getByRole("region", { name: "Shop sent you a code" });
    expect(card).toHaveTextContent("https://shop.example");
  });

  it("asks the phone for the code that came by text", () => {
    renderCard();
    expect(screen.getByLabelText("Code")).toHaveAttribute("autocomplete", "one-time-code");
  });

  it("hands over the code and clears the field", () => {
    const onEnter = renderCard();
    fireEvent.change(screen.getByLabelText("Code"), { target: { value: " 493817 " } });
    fireEvent.click(screen.getByRole("button", { name: "Enter code" }));
    expect(onEnter).toHaveBeenCalledTimes(1);
    expect(onEnter).toHaveBeenCalledWith("493817");
    expect(screen.getByLabelText("Code")).toHaveValue("");
  });

  it("offers nothing to send until a code is typed", () => {
    renderCard();
    expect(screen.getByRole("button", { name: "Enter code" })).toBeDisabled();
  });

  it("an entered card says so and asks for nothing", () => {
    renderCard({ request: { ...REQUEST, status: "entered" } });
    expect(screen.getByRole("status")).toHaveTextContent("Code entered");
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("a lapsed card says nothing was typed", () => {
    renderCard({ request: { ...REQUEST, status: "lapsed" } });
    expect(screen.getByRole("status")).toHaveTextContent("Lapsed. Nothing was typed.");
    expect(screen.queryByRole("textbox")).toBeNull();
  });
});
