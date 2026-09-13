import { describe, expect, it } from "vitest";
import { initials } from "@/components/AccountMenu";

describe("initials", () => {
  it("takes both parts of a dotted address", () => {
    expect(initials("mark.nygaard@hotmail.com")).toBe("MN");
  });

  it("falls back to the first two letters", () => {
    expect(initials("majse@example.com")).toBe("MA");
  });

  it("handles the other separators people use", () => {
    expect(initials("mark_nygaard@example.com")).toBe("MN");
    expect(initials("mark-nygaard@example.com")).toBe("MN");
  });

  it("has something to draw when nobody is signed in", () => {
    // Sign-in can be off entirely, and the menu still holds the
    // appearance setting.
    expect(initials(undefined)).toBe("·");
  });
});
