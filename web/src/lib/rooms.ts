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
  lights: Device[];
  /** How many of them are reporting on. */
  on: number;
  /**
   * What the room is reporting, from whichever device in it reports
   * such a thing. Shown on the card because a room's temperature is
   * the other thing you want to know at a glance.
   */
  temperature?: number;
  humidity?: number;
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
      const lights = all
        .filter(isControllable)
        .sort((a, b) => a.name.localeCompare(b.name));
      return {
        name,
        label: humanize(name),
        lights,
        on: lights.filter((light) => light.state.on === true).length,
        temperature: firstReported(all, "temperature_celsius"),
        humidity: firstReported(all, "humidity_percent"),
      };
    })
    .filter((room) => room.lights.length > 0)
    .sort((a, b) => a.label.localeCompare(b.label));
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
 * Whether a command aimed at `target` should move this device.
 *
 * A room is named by its string; anything else is one device by id.
 */
export function targets(device: Device, target: Device | string): boolean {
  return typeof target === "string"
    ? device.room === target && isControllable(device)
    : device.id === target.id;
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
