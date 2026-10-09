import { describe, expect, it } from "vitest";
import { foundText } from "./SonosPanel";
import type { SonosSpeaker } from "@/lib/api";

function speaker(name: string, answering = true): SonosSpeaker {
  return { id: name, name, ip: "10.0.0.2", home_theater: false, room: null, answering };
}

describe("foundText", () => {
  it("names what the speaker described", () => {
    expect(
      foundText({
        configured: true,
        error: null,
        sonos: [speaker("Living Room"), speaker("Sonos Move"), speaker("Gone", false)],
      }),
    ).toBe("Found 2 Sonos rooms: Living Room, Sonos Move.");
  });

  it("says why when nothing answered", () => {
    expect(foundText({ configured: true, error: "No Sonos answered", sonos: [] })).toBe(
      "No Sonos answered",
    );
  });

  it("says nothing before there is an address", () => {
    expect(foundText({ configured: false, error: null, sonos: [] })).toBeNull();
  });
});
