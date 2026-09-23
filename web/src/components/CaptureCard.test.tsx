import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { CaptureCard, captureSummary } from "@/components/CaptureCard";

const KEPT = {
  id: 1,
  heard_at: "2026-09-23T09:04:14Z",
  transcript: "Thank you.",
  outcome: "dropped",
  bytes: 96_000,
};

describe("CaptureCard", () => {
  it("is off until somebody turns it on", () => {
    // It is a microphone writing down a living room. No amount of
    // usefulness makes that a reasonable default.
    render(<CaptureCard enabled={false} onChange={vi.fn()} />);
    expect(screen.getByRole("switch")).not.toBeChecked();
  });

  it("writes the switch straight through", () => {
    const onChange = vi.fn();
    render(<CaptureCard enabled={false} onChange={onChange} />);
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("says nothing about a pile that is not being collected", () => {
    // Matched on the summary rather than the word "kept", which the
    // card's own description also contains.
    render(
      <CaptureCard enabled={false} captures={[KEPT]} onChange={vi.fn()} />,
    );
    expect(screen.queryByText(/came to nothing/)).not.toBeInTheDocument();
  });

  it("offers no delete button for an empty pile", () => {
    render(
      <CaptureCard
        enabled
        captures={[]}
        onChange={vi.fn()}
        onClear={vi.fn()}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "Delete all" }),
    ).not.toBeInTheDocument();
  });

  it("throws the lot away in one go", () => {
    const onClear = vi.fn();
    render(
      <CaptureCard
        enabled
        captures={[KEPT]}
        onChange={vi.fn()}
        onClear={onClear}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete all" }));
    expect(onClear).toHaveBeenCalled();
  });
});

describe("captureSummary", () => {
  it("counts the ones that came to nothing", () => {
    // The number that matters: a satellite waking fifty times a day
    // and meaning it twice has a wake-word problem, and counting is
    // the only honest way to know.
    const summary = captureSummary([
      KEPT,
      { ...KEPT, id: 2, outcome: "answered", transcript: "lights off" },
      { ...KEPT, id: 3 },
    ]);
    expect(summary).toContain("3 kept");
    expect(summary).toContain("2 came to nothing");
  });

  it("says how much room it is taking", () => {
    expect(captureSummary([KEPT])).toMatch(/0\.1 MB/);
  });

  it("says plainly when there is nothing yet", () => {
    expect(captureSummary([])).toBe("Nothing kept yet.");
  });
});
