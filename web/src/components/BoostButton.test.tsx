import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BoostButton } from "./BoostButton";

describe("BoostButton", () => {
  const handlers = { onBoost: vi.fn(), onResume: vi.fn() };

  it("offers a boost when nothing is running", () => {
    const onBoost = vi.fn();
    render(<BoostButton boosting={false} onBoost={onBoost} onResume={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Boost the heating/ }));
    expect(onBoost).toHaveBeenCalledTimes(1);
  });

  it("offers the way out while one is", () => {
    // The same button, because ending a boost and starting one are one
    // thought — and a second control that means something for half an
    // hour is wrong the rest of the time.
    const onResume = vi.fn();
    render(<BoostButton boosting onBoost={vi.fn()} onResume={onResume} />);
    expect(screen.getByText("Resume")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /put every room back/ }));
    expect(onResume).toHaveBeenCalledTimes(1);
  });

  it("takes no presses while it is asking", () => {
    render(<BoostButton boosting={false} pending {...handlers} />);
    expect(screen.getByRole("button")).toBeDisabled();
  });
});
