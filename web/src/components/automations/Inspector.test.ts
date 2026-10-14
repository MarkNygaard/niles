import { describe, expect, it } from "vitest";
import { TRIGGER_CHOICES, branchKinds } from "./Inspector";

describe("TRIGGER_CHOICES", () => {
  it("offers every kind of trigger, by what it is called", () => {
    expect(TRIGGER_CHOICES.map((c) => c.label)).toEqual([
      "Door or window",
      "Motion",
      "Device turns on or off",
      "A reading changes",
      "Button press",
      "Home or away",
      "Time of day",
      "Sunrise or sunset",
      "Timer goes off",
    ]);
  });
});

describe("branchKinds", () => {
  it("offers an if inside an if until the third level down", () => {
    expect(branchKinds(1)).toContain("if");
    expect(branchKinds(2)).toContain("if");
    expect(branchKinds(3)).not.toContain("if");
    expect(branchKinds(3)).toContain("delay");
  });
});
