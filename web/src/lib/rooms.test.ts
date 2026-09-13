import { describe, expect, it } from "vitest";
import {
  brightnessOf,
  mergeReported,
  optimistic,
  roomSummary,
  roomToggle,
  roomsOf,
  targets,
} from "./rooms";
import type { Device, DeviceState } from "./api";

function state(partial: Partial<DeviceState> = {}): DeviceState {
  return {
    on: null,
    brightness: null,
    color_temp_kelvin: null,
    rgb: null,
    temperature_celsius: null,
    humidity_percent: null,
    battery_percent: null,
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

  it("matches one light by its id", () => {
    expect(targets(lamp, lamp)).toBe(true);
    expect(targets(device("z2m:office/other"), lamp)).toBe(false);
  });

  it("matches every light in a named room", () => {
    expect(targets(lamp, "office")).toBe(true);
    expect(targets(lamp, "kitchen")).toBe(false);
  });

  it("leaves a room's sensors and wall switches alone", () => {
    expect(targets(device("z2m:office/thermometer", { class: "sensor" }), "office")).toBe(
      false,
    );
    expect(targets(device("z2m:office/switch", { class: "switch" }), "office")).toBe(false);
  });

  it("reaches a room's outlets", () => {
    expect(targets(device("z2m:office/corner_lamp", { class: "outlet" }), "office")).toBe(
      true,
    );
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
