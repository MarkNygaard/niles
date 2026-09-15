import { describe, expect, it } from "vitest";
import { heatColor } from "@/lib/heat";

/** The three numbers out of an `oklch(l c h)` string. */
function parse(css: string): { l: number; c: number; h: number } {
  const [l, c, h] = css
    .replace(/^oklch\(|\)$/g, "")
    .split(" ")
    .map(Number);
  return { l, c, h };
}

describe("heatColor", () => {
  it("gives off a colour of its own, off the scale", () => {
    // Off is not the coldest setting, it is the absence of one, so it
    // is grey rather than the blue-green 5° gets.
    const off = parse(heatColor(null));
    expect(off.c).toBeLessThan(0.03);
    const coldest = parse(heatColor(5));
    expect(coldest.c).toBeGreaterThan(0.05);
  });

  it("runs from blue-green up to orange", () => {
    // tado's journey, measured off their own screens: the hue falls the
    // whole way, which is what "cool at the bottom, warm at the top"
    // means in a colour space.
    expect(parse(heatColor(5)).h).toBeCloseTo(185, 0);
    expect(parse(heatColor(25)).h).toBeCloseTo(60, 0);
  });

  it("never turns back on itself", () => {
    // Every half degree up has to look warmer than the one below, or
    // the dial stops reading as a dial.
    let previous = Infinity;
    for (let c = 5; c <= 25; c += 0.5) {
      const hue = parse(heatColor(c)).h;
      expect(hue).toBeLessThanOrEqual(previous);
      previous = hue;
    }
  });

  it("brightens as it warms", () => {
    expect(parse(heatColor(25)).l).toBeGreaterThan(parse(heatColor(5)).l);
  });

  it("turns yellow between 18.5 and 19", () => {
    // Half a degree carrying a whole hue shift is deliberate: it is the
    // one place on the dial where a small move means something.
    const green = parse(heatColor(18.5)).h;
    const yellow = parse(heatColor(19)).h;
    expect(green).toBeGreaterThan(140);
    expect(yellow).toBeLessThan(130);
  });

  it("holds at the ends rather than running past them", () => {
    expect(heatColor(-40)).toBe(heatColor(5));
    expect(heatColor(100)).toBe(heatColor(25));
  });

  it("stays dark enough to read white on", () => {
    // tado's own reaches 0.74 by 18.5°, where white text is 2.18:1 and
    // genuinely hard to read. Ours stops short of that on purpose.
    for (let c = 5; c <= 25; c += 0.5) {
      expect(parse(heatColor(c)).l).toBeLessThanOrEqual(0.66);
    }
  });
});
