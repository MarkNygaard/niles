import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BrightnessBar, fractionOf, percentAt } from "./BrightnessBar";

// jsdom has no layout, so the bar has to be told how wide it is and
// pointer capture has to be stubbed. Worth the trouble: committing on
// release is the whole behaviour, and the keyboard path never sets
// `dragging` — a keyboard-only test passes against a broken drag.
function drag(bar: HTMLElement, toX: number) {
  bar.getBoundingClientRect = () =>
    ({ left: 0, right: 200, width: 200, top: 0, bottom: 48, height: 48 }) as DOMRect;
  bar.setPointerCapture = () => {};
  fireEvent.pointerDown(bar, { clientX: 20, pointerId: 1 });
  fireEvent.pointerMove(bar, { clientX: toX, pointerId: 1 });
  fireEvent.pointerUp(bar, { clientX: toX, pointerId: 1 });
}

describe("the bar's range", () => {
  it("fills to the end at full", () => {
    expect(fractionOf(100)).toBe(1);
  });

  it("still shows something at the dimmest setting", () => {
    // An empty track reads as broken rather than as dim, and the grip
    // has to sit somewhere.
    expect(fractionOf(1)).toBeGreaterThan(0.1);
  });

  it("reads back what it drew", () => {
    for (const percent of [1, 25, 50, 99, 100]) {
      expect(percentAt(fractionOf(percent))).toBe(percent);
    }
  });

  it("holds the ends against a thumb that overshoots", () => {
    expect(percentAt(-0.5)).toBe(1);
    expect(percentAt(1.5)).toBe(100);
  });
});

describe("BrightnessBar", () => {
  const props = { label: "Lamp brightness", color: "#ffaa00" };

  it("sends once, when the drag ends", () => {
    const onCommit = vi.fn();
    render(<BrightnessBar value={20} onCommit={onCommit} {...props} />);
    drag(screen.getByRole("slider", { name: "Lamp brightness" }), 200);
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith(100);
  });

  it("reports every position it passes through", () => {
    // The row prints the percentage next to the bar, and a number that
    // disagrees with the thumb is the number nobody trusts after.
    const onDraft = vi.fn();
    render(<BrightnessBar value={20} onCommit={vi.fn()} onDraft={onDraft} {...props} />);
    drag(screen.getByRole("slider", { name: "Lamp brightness" }), 100);
    expect(onDraft.mock.calls.length).toBeGreaterThan(1);
  });

  it("holds what was asked for while the group catches up", () => {
    // A single bulb echoes one state back. A group reports again as
    // each of its members acks, and again through a fade — every one
    // of those numbers is true and none of them is what was asked for.
    // Taking them made the bar walk back down the fade and up again.
    const { rerender } = render(
      <BrightnessBar value={20} onCommit={vi.fn()} {...props} />,
    );
    const bar = screen.getByRole("slider", { name: "Lamp brightness" });
    drag(bar, 200);
    expect(bar).toHaveAttribute("aria-valuenow", "100");

    // The first member reports, still mid-fade.
    rerender(<BrightnessBar value={62} onCommit={vi.fn()} {...props} />);
    expect(bar).toHaveAttribute("aria-valuenow", "100");
    // And the second, from where it started.
    rerender(<BrightnessBar value={20} onCommit={vi.fn()} {...props} />);
    expect(bar).toHaveAttribute("aria-valuenow", "100");
  });

  it("settles as soon as the light agrees", () => {
    const { rerender } = render(
      <BrightnessBar value={20} onCommit={vi.fn()} {...props} />,
    );
    const bar = screen.getByRole("slider", { name: "Lamp brightness" });
    drag(bar, 200);
    // Within one, because a percentage goes to Z2M as 0–254 and comes
    // back rounded: asking for 100 can honestly answer 99.
    rerender(<BrightnessBar value={99} onCommit={vi.fn()} {...props} />);
    expect(bar).toHaveAttribute("aria-valuenow", "99");

    // And afterwards it follows the light again — the curve and the
    // voice move it too.
    rerender(<BrightnessBar value={40} onCommit={vi.fn()} {...props} />);
    expect(bar).toHaveAttribute("aria-valuenow", "40");
  });

  it("takes the light's word when nothing is being dragged", () => {
    const { rerender } = render(
      <BrightnessBar value={20} onCommit={vi.fn()} {...props} />,
    );
    const bar = screen.getByRole("slider", { name: "Lamp brightness" });
    expect(bar).toHaveAttribute("aria-valuenow", "20");

    // The curve and the voice move lights too, and the bar follows.
    rerender(<BrightnessBar value={80} onCommit={vi.fn()} {...props} />);
    expect(bar).toHaveAttribute("aria-valuenow", "80");
  });

  it("does not answer a pointer when it is disabled", () => {
    const onCommit = vi.fn();
    render(<BrightnessBar value={20} disabled onCommit={onCommit} {...props} />);
    const bar = screen.getByRole("slider", { name: "Lamp brightness" });
    expect(bar).toHaveAttribute("aria-disabled", "true");
    expect(bar).toHaveAttribute("tabindex", "-1");
  });
});
