import { describe, expect, it } from "vitest";
import { keyboardInset } from "@/hooks/useKeyboardInset";

describe("keyboardInset", () => {
  it("is what the keyboard covers on an iPhone", () => {
    // iOS keeps the page at full height and shrinks the visible part.
    expect(keyboardInset(844, { height: 508, offsetTop: 0 })).toBe(336);
  });

  it("allows for Safari having scrolled the page up", () => {
    expect(keyboardInset(844, { height: 508, offsetTop: 120 })).toBe(216);
  });

  it("is nothing where the page shrinks instead, as on Android", () => {
    expect(keyboardInset(508, { height: 508, offsetTop: 0 })).toBe(0);
  });

  it("ignores the browser's own toolbar moving", () => {
    expect(keyboardInset(844, { height: 800, offsetTop: 0 })).toBe(0);
  });
});
