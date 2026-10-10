import { describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { edgeScroll, useGripsHoldStill } from "./drag";

describe("edgeScroll", () => {
  it("scrolls under a drag only at the screen's edges", () => {
    expect(edgeScroll(400, 800)).toBe(0);
    expect(edgeScroll(790, 800)).toBeGreaterThan(0);
    expect(edgeScroll(10, 800)).toBeLessThan(0);
    expect(edgeScroll(799, 800)).toBeGreaterThan(edgeScroll(770, 800));
    // A card low on the screen is not the edge.
    expect(edgeScroll(720, 800)).toBe(0);
  });
});

describe("useGripsHoldStill", () => {
  function touchmoveOn(el: Element): boolean {
    const event = new Event("touchmove", { bubbles: true, cancelable: true });
    el.dispatchEvent(event);
    return event.defaultPrevented;
  }

  it("refuses a touch moving on a handle, and only there", () => {
    const { unmount } = renderHook(() => useGripsHoldStill());
    const grip = document.createElement("button");
    grip.dataset.grip = "";
    const icon = document.createElement("svg");
    grip.append(icon);
    const elsewhere = document.createElement("div");
    document.body.append(grip, elsewhere);
    expect(touchmoveOn(icon)).toBe(true);
    expect(touchmoveOn(elsewhere)).toBe(false);
    unmount();
    expect(touchmoveOn(icon)).toBe(false);
    grip.remove();
    elsewhere.remove();
  });
});
