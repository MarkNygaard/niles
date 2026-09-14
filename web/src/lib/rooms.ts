import type { Device, DeviceState, SetLight } from "@/lib/api";

/** `tv_lightstrip` → `Tv lightstrip`. Ids are snake_case by rule. */
export function humanize(raw: string): string {
  const spaced = raw.replace(/_/g, " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/**
 * Whether this is something the dashboard can switch.
 *
 * Outlets count. A lamp on a smart plug is a light to whoever owns it,
 * and leaving it off the dashboard because Z2M calls it an outlet would
 * make it the one lamp in the house the page can't reach.
 */
export function isControllable(device: Device): boolean {
  return device.class === "light" || device.class === "outlet";
}

/** Whether it takes a level, as opposed to only on and off. */
export function isDimmable(device: Device): boolean {
  return device.class === "light";
}

export interface Room {
  /** The id Niles uses, and what the API is addressed by. */
  name: string;
  label: string;
  /** The ones the source can currently reach. */
  lights: Device[];
  /** How many of them are reporting on. */
  on: number;
  /**
   * The names of the ones it cannot.
   *
   * Left out of `lights` — a control that publishes to something not
   * listening looks broken rather than offline — but named here rather
   * than vanishing, because a light that disappears when its battery
   * dies is a light nobody notices has died.
   */
  unreachable: string[];
  /**
   * What the room is reporting, from whichever device in it reports
   * such a thing. Shown on the card because a room's temperature is
   * the other thing you want to know at a glance.
   */
  temperature?: number;
  humidity?: number;
  /** What is standing open in it. Empty when everything is shut. */
  openings: Opening[];
}

/**
 * Something in a room that is open right now.
 *
 * Deduplicated by kind rather than listed per device: three open
 * windows is still "the windows are open", and three identical icons
 * on a card the size of a thumbnail is noise, not information. The
 * count rides along for the label, which is where it belongs.
 */
export interface Opening {
  kind: "door" | "window";
  count: number;
}

/**
 * The house, grouped the way it is laid out.
 *
 * Rooms without a light are left out. The dashboard is for controlling
 * lights, and a card you can't press is a card that only takes up room.
 */
export function roomsOf(devices: Device[]): Room[] {
  const byRoom = new Map<string, Device[]>();
  for (const device of devices) {
    const existing = byRoom.get(device.room);
    if (existing) existing.push(device);
    else byRoom.set(device.room, [device]);
  }

  return [...byRoom.entries()]
    .map(([name, all]) => {
      const controllable = all
        .filter(isControllable)
        .sort((a, b) => a.name.localeCompare(b.name));
      const lights = controllable.filter((d) => d.available !== false);
      return {
        name,
        label: humanize(name),
        lights,
        unreachable: controllable
          .filter((d) => d.available === false)
          .map((d) => humanize(d.name)),
        on: lights.filter((light) => light.state.on === true).length,
        temperature: firstReported(all, "temperature_celsius"),
        humidity: firstReported(all, "humidity_percent"),
        openings: openingsOf(all),
      };
    })
    .filter((room) => room.lights.length + room.unreachable.length > 0)
    .sort((a, b) => a.label.localeCompare(b.label));
}

/**
 * The open doors and windows in a set of devices.
 *
 * Which of the two it is comes from the *name*, and only because being
 * wrong about it costs an icon rather than a behaviour — Niles already
 * knows it is a contact sensor from what Z2M says it exposes, so a
 * sensor called `garden` still reports, it just draws a door.
 */
export function openingsOf(devices: Device[]): Opening[] {
  const counts = new Map<Opening["kind"], number>();
  for (const device of devices) {
    if (device.state.open !== true) continue;
    const kind: Opening["kind"] = /window/i.test(device.name) ? "window" : "door";
    counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([kind, count]) => ({ kind, count }))
    .sort((a, b) => a.kind.localeCompare(b.kind));
}

function firstReported(
  devices: Device[],
  field: "temperature_celsius" | "humidity_percent",
): number | undefined {
  for (const device of devices) {
    const value = device.state[field];
    if (value !== null && value !== undefined) return value;
  }
  return undefined;
}

/**
 * What pressing the room should do.
 *
 * "Any light on means turn them off" rather than counting a majority:
 * pressing a room with one lamp left on is nearly always someone
 * clearing the room, not someone topping it up.
 */
export function roomToggle(room: Room): SetLight {
  return { on: room.on === 0 };
}

/** How the card reads under the room's name. */
export function roomSummary(room: Room): string {
  if (room.lights.length === 1) {
    return room.on === 1 ? "On" : "Off";
  }
  if (room.on === 0) return `${room.lights.length} lights, all off`;
  if (room.on === room.lights.length) return `All ${room.on} on`;
  return `${room.on} of ${room.lights.length} on`;
}

/**
 * Where a brightness slider should start for a light that is off.
 *
 * A light reports the brightness it was last set to even while off, so
 * that is the honest starting point; full brightness is the fallback
 * for one that has never said.
 */
export function brightnessOf(light: Device): number {
  return light.state.brightness ?? 100;
}

/**
 * What a command is aimed at.
 *
 * Spelled out rather than overloading a string, because there are now
 * three scopes and "is it a string" would have to mean two of them.
 */
export type Target =
  | { scope: "house" }
  | { scope: "room"; room: string }
  | { scope: "light"; light: Device };

/** Whether a command aimed at `target` should move this device. */
export function targets(device: Device, target: Target): boolean {
  switch (target.scope) {
    case "house":
      return isControllable(device);
    case "room":
      return device.room === target.room && isControllable(device);
    case "light":
      return device.id === target.light.id;
  }
}

/**
 * What pressing the whole house should do.
 *
 * The same rule as a room, for the same reason: somebody pressing this
 * with one lamp still burning is clearing the house, not topping it up.
 * And it stays a toggle rather than an off-only button, so a press is
 * always undoable by pressing again.
 */
export function houseToggle(rooms: Room[]): SetLight {
  return { on: !rooms.some((room) => room.on > 0) };
}

/** How the bar above the rooms reads. */
export function houseSummary(rooms: Room[]): string {
  const on = rooms.reduce((total, room) => total + room.on, 0);
  const total = rooms.reduce((count, room) => count + room.lights.length, 0);
  if (on === 0) return `Nothing on, ${total} lights`;
  if (on === total) return `All ${total} on`;
  return `${on} of ${total} on`;
}

/**
 * What the light will look like once the command lands.
 *
 * Shown before the light has answered, so the press looks like it did
 * something. Brightness implies on, because that is what the light
 * does: Z2M and WLED both wake a light that is told a level, and
 * leaving the row dark until the report came back would read as the
 * press being ignored.
 */
export function optimistic(device: Device, body: SetLight): Device {
  const state = { ...device.state };
  if (body.on !== undefined) state.on = body.on;
  // A room-wide command is narrowed per device on the server, so show
  // the same narrowing here rather than an outlet appearing to dim.
  if (!isDimmable(device)) return { ...device, state };
  if (body.brightness !== undefined) {
    state.brightness = body.brightness;
    state.on = true;
  }
  // A light is in colour mode or white mode, never both, so setting
  // one clears the other rather than showing a light as having a
  // colour and a temperature at once.
  if (body.color_temp_kelvin !== undefined) {
    state.color_temp_kelvin = body.color_temp_kelvin;
    state.rgb = null;
  }
  if (body.rgb !== undefined) {
    state.rgb = body.rgb;
    state.color_temp_kelvin = null;
  }
  return { ...device, state };
}

/**
 * Fold a reported change into what we already know.
 *
 * Sources report only the fields that changed, so a frame saying
 * `{on: false}` carries `null` for everything else. Replacing wholesale
 * would erase the brightness the light still has and the row would
 * lose its slider; `null` here means "didn't say", never "cleared".
 */
export function mergeReported(
  known: DeviceState,
  reported: Partial<DeviceState>,
): DeviceState {
  const merged = { ...known };
  for (const [field, value] of Object.entries(reported)) {
    if (value !== null && value !== undefined) {
      (merged as Record<string, unknown>)[field] = value;
    }
  }
  return merged;
}
