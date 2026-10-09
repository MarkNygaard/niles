import { describe, expect, it } from "vitest";
import { mediaOrder, somethingPlays } from "./MediaPage";
import type { RoomMusic } from "@/lib/api";

function room(name: string, playing: boolean): RoomMusic {
  return { room: name, playing, what: null, kind: playing ? "music" : null, volume: 20 };
}

describe("MediaPage", () => {
  it("puts the rooms that play first", () => {
    const ordered = mediaOrder([
      room("walk_in_closet", false),
      room("kitchen", false),
      room("living_room", true),
    ]).map((r) => r.room);
    expect(ordered).toEqual(["living_room", "kitchen", "walk_in_closet"]);
  });

  it("knows when something plays", () => {
    expect(somethingPlays([room("kitchen", false)], false)).toBe(false);
    expect(somethingPlays([room("kitchen", true)], false)).toBe(true);
    expect(somethingPlays([], true)).toBe(true);
    expect(somethingPlays(undefined, undefined)).toBe(false);
  });
});
