import { describe, expect, it } from "vitest";
import { dragWheel, keyOf, liveUrl, modifiersOf, pageAt, tap } from "@/lib/browser";

const PAGE = { width: 1280, height: 900 };

describe("pageAt", () => {
  it("scales a point on the picture back to the page", () => {
    // The page drawn at half size, 20 px in from the left and 100 down.
    const shown = { left: 20, top: 100, width: 640, height: 450 };
    expect(pageAt(20, 100, shown, PAGE)).toEqual({ x: 0, y: 0 });
    expect(pageAt(340, 325, shown, PAGE)).toEqual({ x: 640, y: 450 });
    expect(pageAt(660, 550, shown, PAGE)).toEqual({ x: 1280, y: 900 });
  });

  it("is nothing off the picture or for a picture not drawn yet", () => {
    const shown = { left: 0, top: 0, width: 640, height: 450 };
    expect(pageAt(-1, 10, shown, PAGE)).toBeNull();
    expect(pageAt(10, 451, shown, PAGE)).toBeNull();
    expect(pageAt(10, 10, { ...shown, width: 0 }, PAGE)).toBeNull();
  });
});

describe("keyOf", () => {
  it("passes on named keys and single characters, and nothing else", () => {
    expect(keyOf("Enter")).toBe("Enter");
    expect(keyOf("ArrowLeft")).toBe("ArrowLeft");
    expect(keyOf("a")).toBe("a");
    expect(keyOf("æ")).toBe("æ");
    expect(keyOf("Shift")).toBeNull();
    expect(keyOf("F12")).toBeNull();
    expect(keyOf("Unidentified")).toBeNull();
  });
});

describe("modifiersOf", () => {
  it("is the browser's bit mask", () => {
    const none = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false };
    expect(modifiersOf(none)).toBe(0);
    expect(modifiersOf({ ...none, ctrlKey: true, shiftKey: true })).toBe(10);
    expect(modifiersOf({ altKey: true, ctrlKey: true, metaKey: true, shiftKey: true })).toBe(15);
  });
});

describe("dragWheel", () => {
  it("scrolls down when a finger drags up, as a phone does", () => {
    expect(dragWheel({ x: 100, y: 500 }, { x: 100, y: 300 })).toEqual({ dx: 0, dy: 200 });
  });
});

describe("tap", () => {
  it("is the key going down and coming up", () => {
    expect(tap("Backspace").map((k) => k.type === "key" && k.press)).toEqual(["down", "up"]);
  });
});

describe("liveUrl", () => {
  it("is a secure socket on a secure page, on the page's own host", () => {
    expect(liveUrl({ protocol: "https:", host: "niles.example" })).toBe(
      "wss://niles.example/me/browser/live",
    );
    expect(liveUrl({ protocol: "http:", host: "localhost:5173" })).toBe(
      "ws://localhost:5173/me/browser/live",
    );
  });
});
