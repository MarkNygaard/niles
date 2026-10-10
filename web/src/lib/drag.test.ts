import { describe, expect, it } from "vitest";
import { edgeScroll } from "./drag";

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
