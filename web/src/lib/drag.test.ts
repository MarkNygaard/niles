import { describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { edgeScroll, useGripsHoldStill, visibleBottom } from "./drag";

describe("edgeScroll", () => {
  it("scrolls under a drag only at the screen's edges", () => {
    expect(edgeScroll(400, 800)).toBe(0);
    expect(edgeScroll(790, 800)).toBeGreaterThan(0);
    expect(edgeScroll(10, 800)).toBeLessThan(0);
    expect(edgeScroll(799, 800)).toBeGreaterThan(edgeScroll(770, 800));
    // A card low on the screen is not the edge.
    expect(edgeScroll(720, 800)).toBe(0);
  });

  it("scrolls fastest over what covers the bottom of the page", () => {
    // Visible down to 700, a tab bar below.
    expect(edgeScroll(690, 700)).toBeGreaterThan(0);
    expect(edgeScroll(780, 700)).toBe(edgeScroll(700, 700));
    expect(edgeScroll(780, 700)).toBe(12);
  });
});

describe("visibleBottom", () => {
  it("is the window's bottom without a tab bar over it", () => {
    expect(visibleBottom()).toBe(window.innerHeight);
  });

  it("is the top of a tab bar fixed over the page", () => {
    const bar = document.createElement("nav");
    bar.setAttribute("aria-label", "Main");
    bar.style.position = "fixed";
    bar.getBoundingClientRect = () => ({ top: 700, height: 90 }) as DOMRect;
    document.body.append(bar);
    expect(visibleBottom()).toBe(700);
    bar.remove();
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
