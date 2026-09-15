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
    expect(parse(heatColor(5)).h).toBeCloseTo(193, 0);
    expect(parse(heatColor(25)).h).toBeCloseTo(43, 0);
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

  it("brightens as it warms, then falls back for the orange", () => {
    // Not monotonic, and deliberately so: yellow is simply a light
    // colour, and forcing it down the way the rest of the scale
    // suggests would produce olive rather than yellow.
    expect(parse(heatColor(19)).l).toBeGreaterThan(parse(heatColor(5)).l);
    expect(parse(heatColor(25)).l).toBeLessThan(parse(heatColor(19)).l);
  });

  it("turns yellow between 18.5 and 19", () => {
    // Half a degree carrying a whole hue shift is deliberate: it is the
    // one place on the dial where a small move means something.
    const green = parse(heatColor(18.5)).h;
    const yellow = parse(heatColor(19)).h;
    expect(green).toBeGreaterThan(140);
    expect(yellow).toBeLessThan(100);
  });

  it("holds at the ends rather than running past them", () => {
    expect(heatColor(-40)).toBe(heatColor(5));
    expect(heatColor(100)).toBe(heatColor(25));
  });

  it("reaches tado's own colours at the stops", () => {
    // Measured out of their screenshots rather than guessed — two of
    // the guesses were wrong by 30 degrees of hue.
    expect(parse(heatColor(5))).toMatchObject({ l: 0.622, h: 193 });
    expect(parse(heatColor(19))).toMatchObject({ l: 0.853, h: 90 });
    expect(parse(heatColor(25))).toMatchObject({ l: 0.674, h: 43 });
  });
});
