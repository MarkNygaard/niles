import { describe, expect, it } from "vitest";
import { menuOf } from "./menu";

describe("menuOf", () => {
  it("shows everything in its first order when nothing is arranged", () => {
    expect(menuOf({})).toEqual([
      { id: "groceries", hidden: false },
      { id: "chat", hidden: false },
    ]);
  });

  it("follows the arranged order and hides what is hidden", () => {
    expect(menuOf({ menu: { order: ["chat", "groceries"], hidden: ["groceries"] } })).toEqual([
      { id: "chat", hidden: false },
      { id: "groceries", hidden: true },
    ]);
  });

  it("puts an entry the order leaves out after the ones it names", () => {
    // A page added later has to turn up somewhere.
    expect(menuOf({ menu: { order: ["chat"] } }).map((m) => m.id)).toEqual(["chat", "groceries"]);
  });

  it("ignores names it does not know and names given twice", () => {
    expect(menuOf({ menu: { order: ["me", "chat", "chat", 3] } }).map((m) => m.id)).toEqual([
      "chat",
      "groceries",
    ]);
  });
});
