import { describe, expect, it } from "vitest";
import { menuOf } from "./menu";

describe("menuOf", () => {
  it("shows everything in its first order when nothing is arranged", () => {
    expect(menuOf({})).toEqual([
      { id: "groceries", hidden: false },
      { id: "chat", hidden: false },
      { id: "media", hidden: false },
    ]);
  });

  it("follows the arranged order and hides what is hidden", () => {
    expect(menuOf({ menu: { order: ["chat", "groceries"], hidden: ["groceries"] } })).toEqual([
      { id: "chat", hidden: false },
      { id: "groceries", hidden: true },
      { id: "media", hidden: false },
    ]);
  });

  it("puts an entry the order leaves out after the ones it names", () => {
    // A page added later has to turn up somewhere.
    expect(menuOf({ menu: { order: ["chat"] } }).map((m) => m.id)).toEqual(["chat", "groceries", "media"]);
  });

  it("ignores names it does not know and names given twice", () => {
    expect(menuOf({ menu: { order: ["me", "chat", "chat", 3] } }).map((m) => m.id)).toEqual([
      "chat",
      "groceries",
      "media",
    ]);
  });

  it("shows Media while something plays, unless told otherwise", () => {
    const media = (effective: unknown, playing?: boolean) =>
      menuOf(effective, playing).find((m) => m.id === "media")!.hidden;
    expect(media({}, false)).toBe(true);
    expect(media({}, true)).toBe(false);
    expect(media({ menu: { media: "always" } }, false)).toBe(false);
    expect(media({ menu: { media: "never" } }, true)).toBe(true);
    // Settings lists it whatever plays.
    expect(media({}, undefined)).toBe(false);
  });
});
