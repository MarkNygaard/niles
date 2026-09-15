import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  UnplacedNotice,
  ZonePairing,
  describe as describeZone,
  roomOptions,
  unplaced,
} from "@/components/ZonePairing";
import type { Zone } from "@/lib/api";

function zone(overrides: Partial<Zone> = {}): Zone {
  return {
    id: 6,
    name: "Living Room",
    room: "living_room",
    temperature: 23.97,
    humidity: 60.6,
    target: null,
    on: false,
    overridden: true,
    reachable: true,
    placed_by: "name",
    ...overrides,
  };
}

// The dropdown cannot be opened in jsdom — Base UI's Select hangs it —
// so what it offers is decided by a function a test can call.
describe("roomOptions", () => {
  it("offers the rooms Niles knows about", () => {
    expect(roomOptions(["kitchen", "living_room"], null)).toEqual([
      "kitchen",
      "living_room",
    ]);
  });

  it("keeps a paired room that has no devices in it", () => {
    // A radiator in a room with no lights is still a room, and dropping
    // it would make the box claim the zone is somewhere it is not.
    expect(roomOptions(["kitchen"], "utility")).toEqual(["kitchen", "utility"]);
  });
});

describe("describe", () => {
  it("says off rather than nought degrees", () => {
    // tado sends no target at all for a zone that is off, and "0°"
    // would read as somebody having asked for that.
    expect(describeZone(zone({ on: false, target: null }))).toBe(
      "24.0° · off · overridden",
    );
  });

  it("says what it is heating towards", () => {
    expect(
      describeZone(
        zone({ on: true, target: 23, overridden: false, temperature: 19.24 }),
      ),
    ).toBe("19.2° · heating to 23.0°");
  });

  it("says nothing about a temperature it cannot vouch for", () => {
    // An unreachable valve has no current reading — tado keeps sending
    // the last one it heard, and the server drops it for that reason.
    expect(
      describeZone(zone({ reachable: false, temperature: null })),
    ).toBe("Not answering");
  });
});

describe("ZonePairing", () => {
  it("shows nothing when tado reports no zones", () => {
    const { container } = render(
      <ZonePairing zones={[]} rooms={["kitchen"]} onPair={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("lists a zone with a room to pick", () => {
    render(
      <ZonePairing
        zones={[zone()]}
        rooms={["living_room"]}
        onPair={vi.fn()}
      />,
    );
    expect(screen.getByText("Living Room")).toBeInTheDocument();
    expect(
      screen.getByRole("combobox", { name: "Room for Living Room" }),
    ).toBeInTheDocument();
  });
});

describe("unplaced", () => {
  it("counts only the ones nothing placed", () => {
    // A name match is a guess, but it is a working one — it does not
    // need a person.
    expect(
      unplaced([
        zone({ placed_by: "name" }),
        zone({ id: 2, placed_by: "paired" }),
        zone({ id: 3, placed_by: "nowhere", room: null }),
      ]),
    ).toBe(1);
  });

  it("says nothing at all when every zone has a room", () => {
    const { container } = render(
      <UnplacedNotice zones={[zone({ placed_by: "paired" })]} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("says what an unplaced zone costs", () => {
    render(<UnplacedNotice zones={[zone({ placed_by: "nowhere" })]} />);
    expect(
      screen.getByText(/will not appear on the dashboard/),
    ).toBeInTheDocument();
  });
});
