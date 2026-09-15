import { describe, expect, it } from "vitest";
import {
  brightnessOf,
  mergeReported,
  optimistic,
  roomSummary,
  roomToggle,
  houseSummary,
  houseToggle,
  roomsOf,
  openingsOf,
  subtitle,
  targets,
  humid,
  measured,
  boosted,
  boostEndsAt,
} from "./rooms";
import type { Room } from "./rooms";
import type { Device, DeviceState, Zone } from "./api";

function state(partial: Partial<DeviceState> = {}): DeviceState {
  return {
    on: null,
    brightness: null,
    color_temp_kelvin: null,
    rgb: null,
    temperature_celsius: null,
    humidity_percent: null,
    battery_percent: null,
    open: null,
    ...partial,
  };
}

function device(
  id: string,
  overrides: Omit<Partial<Device>, "state"> & { state?: Partial<DeviceState> } = {},
): Device {
  const [source, rest] = id.split(":");
  const [room, name] = rest.split("/");
  return {
    id,
    source,
    room,
    name,
    class: "light",
    supports_rgb: false,
    supports_color_temp: false,
    ...overrides,
    state: state(overrides.state),
  };
}

describe("roomsOf and reachability", () => {
  it("keeps a light out of the controls when the source cannot reach it", () => {
    // Z2M draws it with a red border; Niles was showing it as on at
    // 100% with a working slider, because the last state it published
    // is the last state anybody ever hears.
    const [room] = roomsOf([
      device("z2m:living_room/bulb_1", { state: { on: true } }),
      device("z2m:living_room/lightstip", {
        available: false,
        state: { on: true },
      }),
    ]);
    expect(room.lights.map((l) => l.name)).toEqual(["bulb_1"]);
    expect(room.unreachable).toEqual(["Lightstip"]);
  });

  it("does not count an unreachable light as on", () => {
    const [room] = roomsOf([
      device("z2m:living_room/bulb_1", { state: { on: true } }),
      device("z2m:living_room/lightstip", {
        available: false,
        state: { on: true },
      }),
    ]);
    expect(room.on).toBe(1);
    expect(roomSummary(room)).toBe("On");
  });

  it("keeps a room whose every light is unreachable", () => {
    // Dropping it would be the silent disappearance the flag exists to
    // avoid, one level up.
    const rooms = roomsOf([
      device("z2m:shed/lamp", { available: false, state: { on: false } }),
    ]);
    expect(rooms).toHaveLength(1);
    expect(rooms[0].lights).toHaveLength(0);
    expect(rooms[0].unreachable).toEqual(["Lamp"]);
  });

  it("treats a device that has said nothing about it as reachable", () => {
    // Availability is optional in Z2M. A house that never switched it
    // on must not lose its dashboard.
    const [room] = roomsOf([
      device("z2m:living_room/bulb_1", { state: { on: true } }),
    ]);
    expect(room.lights).toHaveLength(1);
    expect(room.unreachable).toEqual([]);
  });
});

describe("roomsOf", () => {
  it("groups lights under the room they are in", () => {
    const rooms = roomsOf([
      device("z2m:kitchen/ceiling"),
      device("z2m:kitchen/counter"),
      device("z2m:office/desk_lamp"),
    ]);
    expect(rooms.map((r) => r.name)).toEqual(["kitchen", "office"]);
    expect(rooms[0].lights).toHaveLength(2);
  });

  it("counts a lamp on a smart plug as one of the room's lights", () => {
    // It is a light to whoever owns it; leaving it off because Z2M
    // calls it an outlet would make it the one lamp the page can't
    // reach.
    const rooms = roomsOf([
      device("z2m:living_room/floor_lamp"),
      device("z2m:living_room/corner_lamp", { class: "outlet", state: { on: true } }),
    ]);
    expect(rooms[0].lights).toHaveLength(2);
    expect(rooms[0].on).toBe(1);
  });

  it("leaves out a room with nothing to control", () => {
    // A card you can't press is a card that only takes up room.
    const rooms = roomsOf([
      device("z2m:hallway/thermometer", { class: "sensor" }),
      device("z2m:office/desk_lamp"),
    ]);
    expect(rooms.map((r) => r.name)).toEqual(["office"]);
  });

  it("counts only the lights that say they are on", () => {
    const rooms = roomsOf([
      device("z2m:kitchen/a", { state: { on: true } }),
      device("z2m:kitchen/b", { state: { on: false } }),
      device("z2m:kitchen/c"),
    ]);
    expect(rooms[0].on).toBe(1);
  });

  it("takes the room's temperature from whatever in it reports one", () => {
    const rooms = roomsOf([
      device("z2m:kitchen/ceiling"),
      device("z2m:kitchen/thermometer", {
        class: "sensor",
        state: { temperature_celsius: 21.4, humidity_percent: 54 },
      }),
    ]);
    expect(rooms[0].temperature).toBe(21.4);
    expect(rooms[0].humidity).toBe(54);
  });

  it("gives rooms and their lights a stable order", () => {
    const rooms = roomsOf([
      device("z2m:office/zebra"),
      device("z2m:office/apple"),
      device("z2m:bedroom/lamp"),
    ]);
    expect(rooms.map((r) => r.label)).toEqual(["Bedroom", "Office"]);
    expect(rooms[1].lights.map((l) => l.name)).toEqual(["apple", "zebra"]);
  });

  it("reads snake_case ids back as words", () => {
    const rooms = roomsOf([device("z2m:living_room/tv_lightstrip")]);
    expect(rooms[0].label).toBe("Living room");
  });
});

describe("roomToggle", () => {
  it("turns a dark room on", () => {
    const [room] = roomsOf([device("z2m:kitchen/a", { state: { on: false } })]);
    expect(roomToggle(room)).toEqual({ on: true });
  });

  it("turns the room off when even one light is on", () => {
    // Pressing a room with one lamp left burning is someone clearing
    // the room, not someone topping it up.
    const [room] = roomsOf([
      device("z2m:kitchen/a", { state: { on: true } }),
      device("z2m:kitchen/b", { state: { on: false } }),
    ]);
    expect(roomToggle(room)).toEqual({ on: false });
  });

  it("treats a light that has never reported as not on", () => {
    const [room] = roomsOf([device("z2m:kitchen/a")]);
    expect(roomToggle(room)).toEqual({ on: true });
  });
});

describe("roomSummary", () => {
  const summarise = (...on: boolean[]) =>
    roomSummary(
      roomsOf(on.map((state, i) => device(`z2m:kitchen/l${i}`, { state: { on: state } })))[0],
    );

  it("does not count a room with one light", () => {
    expect(summarise(true)).toBe("On");
    expect(summarise(false)).toBe("Off");
  });

  it("says how many are on when some are", () => {
    expect(summarise(true, false, false)).toBe("1 of 3 on");
  });

  it("says so plainly when all or none are", () => {
    expect(summarise(false, false)).toBe("2 lights, all off");
    expect(summarise(true, true)).toBe("All 2 on");
  });
});

describe("brightnessOf", () => {
  it("uses the level the light last reported, even while off", () => {
    const light = device("z2m:kitchen/a", { state: { on: false, brightness: 30 } });
    expect(brightnessOf(light)).toBe(30);
  });

  it("falls back to full for a light that has never said", () => {
    expect(brightnessOf(device("z2m:kitchen/a"))).toBe(100);
  });
});

describe("targets", () => {
  const lamp = device("z2m:office/desk_lamp");
  const room = (name: string) => ({ scope: "room", room: name }) as const;
  const house = { scope: "house" } as const;

  it("matches one light by its id", () => {
    expect(targets(lamp, { scope: "light", light: lamp })).toBe(true);
    expect(
      targets(device("z2m:office/other"), { scope: "light", light: lamp }),
    ).toBe(false);
  });

  it("matches every light in a named room", () => {
    expect(targets(lamp, room("office"))).toBe(true);
    expect(targets(lamp, room("kitchen"))).toBe(false);
  });

  it("matches every light anywhere when the house is the target", () => {
    expect(targets(lamp, house)).toBe(true);
    expect(targets(device("z2m:kitchen/ceiling"), house)).toBe(true);
  });

  it("leaves a room's sensors and wall switches alone", () => {
    expect(
      targets(device("z2m:office/thermometer", { class: "sensor" }), room("office")),
    ).toBe(false);
    expect(
      targets(device("z2m:office/switch", { class: "switch" }), room("office")),
    ).toBe(false);
  });

  it("leaves sensors and wall switches alone for the house too", () => {
    // The scope is wider; the rule about what a light is, is not.
    expect(targets(device("z2m:all/bedroom_switch", { class: "switch" }), house)).toBe(
      false,
    );
    expect(targets(device("z2m:hallway/thermometer", { class: "sensor" }), house)).toBe(
      false,
    );
  });

  it("reaches a room's outlets", () => {
    expect(
      targets(device("z2m:office/corner_lamp", { class: "outlet" }), room("office")),
    ).toBe(true);
  });
});

describe("the whole house", () => {
  const house = (...on: boolean[]) =>
    roomsOf(
      on.map((state, i) =>
        device(`z2m:room${i}/light`, { state: { on: state } }),
      ),
    );

  it("goes off when anything at all is on", () => {
    // Somebody pressing this with one lamp still burning is clearing
    // the house, not topping it up.
    expect(houseToggle(house(true, false, false))).toEqual({ on: false });
  });

  it("goes on when the house is dark", () => {
    expect(houseToggle(house(false, false))).toEqual({ on: true });
  });

  it("stays a toggle so a press can be undone", () => {
    // An off-only button would be a dead end on the second press.
    expect(houseToggle(house(true))).toEqual({ on: false });
    expect(houseToggle(house(false))).toEqual({ on: true });
  });

  it("counts across rooms rather than counting rooms", () => {
    expect(houseSummary(house(true, false, false))).toBe("1 of 3 on");
    expect(houseSummary(house(true, true))).toBe("All 2 on");
    expect(houseSummary(house(false, false))).toBe("Nothing on, 2 lights");
  });

  it("says nothing is on for a house with no rooms yet", () => {
    expect(houseToggle([])).toEqual({ on: true });
  });
});

describe("optimistic", () => {
  it("shows a brightness as also turning the light on", () => {
    // Z2M and WLED both wake a light told a level, so leaving the row
    // dark would read as the press being ignored.
    const next = optimistic(device("z2m:kitchen/a", { state: { on: false } }), {
      brightness: 40,
    });
    expect(next.state.on).toBe(true);
    expect(next.state.brightness).toBe(40);
  });

  it("keeps a light in one colour mode at a time", () => {
    const lit = device("z2m:kitchen/a", { state: { color_temp_kelvin: 2700 } });
    expect(optimistic(lit, { rgb: [255, 0, 0] }).state).toMatchObject({
      rgb: [255, 0, 0],
      color_temp_kelvin: null,
    });

    const coloured = device("z2m:kitchen/a", { state: { rgb: [255, 0, 0] } });
    expect(optimistic(coloured, { color_temp_kelvin: 4000 }).state).toMatchObject({
      rgb: null,
      color_temp_kelvin: 4000,
    });
  });

  it("does not show a plug dimming", () => {
    // A room-wide brightness is narrowed per device on the server, so
    // the row must not claim a level the plug never received.
    const plug = device("z2m:living_room/corner_lamp", {
      class: "outlet",
      state: { on: false },
    });
    const next = optimistic(plug, { on: true, brightness: 40 });
    expect(next.state.on).toBe(true);
    expect(next.state.brightness).toBeNull();
  });

  it("leaves what the command didn't mention untouched", () => {
    const light = device("z2m:kitchen/a", { state: { on: true, brightness: 80 } });
    expect(optimistic(light, { on: false }).state.brightness).toBe(80);
  });
});

describe("mergeReported", () => {
  it("keeps what the report didn't mention", () => {
    // Sources report only what changed, so a bare {on: false} frame
    // must not erase the brightness the light still has.
    const merged = mergeReported(state({ on: true, brightness: 70 }), {
      on: false,
    });
    expect(merged).toMatchObject({ on: false, brightness: 70 });
  });

  it("takes a reported false as a value, not as absence", () => {
    const merged = mergeReported(state({ on: true }), { on: false });
    expect(merged.on).toBe(false);
  });

  it("ignores the nulls that fill out every frame", () => {
    const merged = mergeReported(state({ brightness: 70 }), {
      on: true,
      brightness: null,
    });
    expect(merged.brightness).toBe(70);
  });
});

describe("openingsOf", () => {
  const shut = (id: string) =>
    device(id, { class: "contact", state: { open: false } });
  const ajar = (id: string) =>
    device(id, { class: "contact", state: { open: true } });

  it("reads the door from what it is, not what it is called", () => {
    // Niles knows it is a contact sensor from Z2M's exposes, so one
    // nobody named "door" still reports. The name only decides which
    // of two icons gets drawn.
    expect(openingsOf([ajar("z2m:office/garden")])).toEqual([
      { kind: "door", count: 1 },
    ]);
  });

  it("calls anything with window in its name a window", () => {
    expect(openingsOf([ajar("z2m:office/bay_window")])).toEqual([
      { kind: "window", count: 1 },
    ]);
  });

  it("counts rather than repeating a kind", () => {
    expect(
      openingsOf([ajar("z2m:office/bay_window"), ajar("z2m:office/side_window")]),
    ).toEqual([{ kind: "window", count: 2 }]);
  });

  it("ignores what is shut and what has never said", () => {
    expect(
      openingsOf([shut("z2m:office/door"), device("z2m:office/hatch", { class: "contact" })]),
    ).toEqual([]);
  });
});

describe("subtitle", () => {
  const lamps = roomsOf([
    device("z2m:kitchen/ceiling", { state: { on: true } }),
    device("z2m:kitchen/counter", { state: { on: true } }),
  ])[0];

  const withZone = (zone: Partial<Zone>): Room => ({
    ...lamps,
    zone: {
      id: 1,
      name: "Kitchen",
      room: "kitchen",
      temperature: 21,
      humidity: 44,
      target: 21.5,
      on: true,
      overridden: false,
      reachable: true,
      until: null,
      placed_by: "paired",
      ...zone,
    },
  });

  it("gives the line to the lights when there is no heating", () => {
    expect(subtitle(lamps)).toBe("All 2 on");
  });

  it("says what the heating was told to do", () => {
    expect(subtitle(withZone({}))).toBe("Set to 21.5°");
  });

  it("names the floor a zone that is off still holds", () => {
    expect(subtitle(withZone({ on: false }))).toBe("Frost protection");
  });

  it("falls back to the lights rather than report a silent valve", () => {
    // Its last known setting is not the room's setting any more, and
    // saying so on the card leaves the card saying nothing at all.
    expect(subtitle(withZone({ reachable: false }))).toBe("All 2 on");
  });
});

describe("a room's readings", () => {
  const room = (
    devices: Device[],
    zone?: Partial<Zone>,
  ): Room => ({
    ...roomsOf([device("z2m:kitchen/ceiling", { state: { on: true } }), ...devices])[0],
    zone: zone && ({
      id: 1,
      name: "Kitchen",
      room: "kitchen",
      temperature: 21,
      humidity: 44,
      target: 21.5,
      on: true,
      overridden: false,
      reachable: true,
      until: null,
      placed_by: "paired",
      ...zone,
    } as Zone),
  });

  const thermometer = device("z2m:kitchen/sensor", {
    class: "sensor",
    state: { temperature_celsius: 19.5, humidity_percent: 61 },
  });

  it("takes both readings from the valve that is heating the room", () => {
    // Half of them used to come from the valve and half from a sensor,
    // so a room tado heats and nothing else measures showed a
    // temperature with no humidity beside it.
    const kitchen = room([], {});
    expect(measured(kitchen)).toBe(21);
    expect(humid(kitchen)).toBe(44);
  });

  it("falls back to a sensor when there is no valve", () => {
    const kitchen = room([thermometer]);
    expect(measured(kitchen)).toBe(19.5);
    expect(humid(kitchen)).toBe(61);
  });

  it("lets the sensor answer for a valve that is not answering", () => {
    const kitchen = room([thermometer], { temperature: null, humidity: null });
    expect(measured(kitchen)).toBe(19.5);
    expect(humid(kitchen)).toBe(61);
  });
});

describe("a boost that is still running", () => {
  const zone = (id: number, until: string | null): Zone => ({
    id,
    name: `Zone ${id}`,
    room: null,
    temperature: 21,
    humidity: 44,
    target: 25,
    on: true,
    overridden: until !== null,
    reachable: true,
    until,
    placed_by: "paired",
  });

  const noon = Date.parse("2026-09-15T12:00:00Z");
  const soon = "2026-09-15T12:20:00Z";
  const later = "2026-09-15T12:30:00Z";

  it("is the zones whose timer has not run out", () => {
    expect(boosted([zone(1, soon), zone(2, later)], noon)).toEqual([1, 2]);
  });

  it("leaves out a room somebody set by hand", () => {
    // Niles writes an end time on nothing else, so a zone without one
    // is not part of a boost and ending one must not undo it.
    expect(boosted([zone(1, soon), zone(2, null)], noon)).toEqual([1]);
  });

  it("forgets a timer that has already passed", () => {
    const gone = "2026-09-15T11:59:00Z";
    expect(boosted([zone(1, gone)], noon)).toEqual([]);
    expect(boostEndsAt([zone(1, gone)], noon)).toBeNull();
  });

  it("ends when the first of them ends", () => {
    // The page puts the button back at that moment rather than at the
    // next poll, so it cannot go on offering to end something that is
    // already over.
    expect(boostEndsAt([zone(1, later), zone(2, soon)], noon)).toBe(
      Date.parse(soon),
    );
  });
});

describe("the order rooms come out in", () => {
  const house = [
    device("z2m:office/lamp", { state: { on: true } }),
    device("z2m:bedroom/lamp", { state: { on: true } }),
    device("z2m:living_room/lamp", { state: { on: true } }),
  ];

  it("falls back to the alphabet when nobody has arranged them", () => {
    expect(roomsOf(house).map((r) => r.name)).toEqual([
      "bedroom",
      "living_room",
      "office",
    ]);
  });

  it("puts the arranged rooms first, in the order they were arranged", () => {
    expect(roomsOf(house, [], ["living_room", "office"]).map((r) => r.name)).toEqual([
      "living_room",
      "office",
      "bedroom",
    ]);
  });

  it("keeps a room nobody has arranged rather than dropping it", () => {
    // A light paired into a new room has to turn up somewhere, and
    // last is the answer that does not make it look lost.
    const named = roomsOf(house, [], ["office"]).map((r) => r.name);
    expect(named[0]).toBe("office");
    expect(named).toContain("bedroom");
    expect(named).toContain("living_room");
  });

  it("ignores a room in the order that no longer exists", () => {
    expect(roomsOf(house, [], ["attic", "office"]).map((r) => r.name)).toEqual([
      "office",
      "bedroom",
      "living_room",
    ]);
  });
});
