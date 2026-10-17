import { describe, expect, it } from "vitest";
import {
  BLINDS_POSITION,
  measureChoices,
  MEASURE_HINT,
  addAction,
  addTrigger,
  blindsPosition,
  blindsTarget,
  daysText,
  defaultAction,
  defaultCondition,
  defaultTrigger,
  deviceChoices,
  deviceLabel,
  lastRunText,
  move,
  moveStep,
  newRule,
  nextTriggerId,
  problems,
  ranAt,
  roomChoices,
  runTrigger,
  sensorChoices,
  siblings,
  stepAt,
  stepsOf,
  summary,
  toFlow,
  toggleDay,
  triggerChoices,
  wakeChoices,
  wakeTarget,
  watchesSeveral,
  withBlindsTarget,
  withSeveralSensors,
  withStep,
  withWakeTarget,
  withTriggerIds,
  withoutStep,
} from "./automations";
import type { Draft, Step } from "./automations";
import type { Action, Device, DeviceState, Run, Trigger } from "./api";

const quiet: DeviceState = {
  on: null,
  brightness: null,
  color_temp_kelvin: null,
  rgb: null,
  temperature_celsius: null,
  humidity_percent: null,
  battery_percent: null,
  open: null,
  motion: null,
  illuminance_lux: null,
  position: null,
};

function device(id: string, cls: string, state: Partial<DeviceState> = {}): Device {
  const [source, rest] = id.split(":");
  const [room, name] = rest.split("/");
  return {
    id,
    source,
    room,
    name,
    class: cls,
    state: { ...quiet, ...state },
    supports_rgb: false,
    supports_color_temp: false,
  };
}

// The house as it is: the office door reports `open` but no class yet.
const devices = [
  device("z2m:office/door", "unknown", { open: false }),
  device("z2m:office/go", "unknown"),
  device("z2m:office/desk_lamp", "light"),
  device("z2m:living_room/lamp", "light"),
  device("z2m:kitchen/kettle", "outlet"),
  device("z2m:hall/front_door", "contact"),
  device("wled:living_room/tv", "light"),
  device("z2m:office/sensor", "unknown", { temperature_celsius: 21, humidity_percent: 40 }),
  device("z2m:hall/motion", "motion", { motion: false, illuminance_lux: 12 }),
];

const garden: Draft = {
  description: "Garden door",
  enabled: true,
  triggers: [{ id: "t1", type: "contact", device: "office/door", open: true }],
  conditions: [
    { kind: "time_of_day", after: "07:00", before: "23:00" },
    { kind: "device_is", device: "office/desk_lamp", on: true },
  ],
  actions: [
    { do: "set_room", room: "office", on: false },
    { do: "notify", body: "The garden door is open" },
  ],
};

// The rule ARCHITECTURE.md shows: two ways in, one branch for each.
const branching: Draft = {
  description: "Office lights and the garden door",
  enabled: true,
  triggers: [
    { id: "opened", type: "contact", device: "office/door", open: true },
    {
      id: "left-open",
      type: "contact",
      device: "office/door",
      open: true,
      for: { minutes: 10 },
    },
  ],
  conditions: [{ kind: "sun", after: "sunset", before: "sunrise" }],
  actions: [
    {
      do: "if",
      conditions: [{ kind: "triggered_by", triggers: ["opened"] }],
      then: [{ do: "set_room", room: "office", on: false }],
      else: [{ do: "notify", body: "The garden door has been open for ten minutes" }],
    },
    { do: "delay", minutes: 5 },
  ],
};

// An `if` in an `if`, so paths go more than one deep.
const nested: Draft = {
  ...branching,
  actions: [
    {
      do: "if",
      conditions: [{ kind: "presence_is", state: "home" }],
      then: [
        {
          do: "if",
          conditions: [{ kind: "triggered_by", triggers: ["left-open"] }],
          then: [],
          else: [
            { do: "set_device", device: "office/desk_lamp", on: true },
            { do: "set_room", room: "office", on: false },
          ],
        },
      ],
    },
  ],
};

const only = (rule: Draft, id: string) => stepAt(rule, id)!;
const edgePairs = (rule: Draft) =>
  toFlow(rule).edges.map((e) => [e.source, e.target, e.label ?? null]);

describe("toFlow", () => {
  it("reads down: the trigger, the conditions, then the actions", () => {
    const { nodes, edges } = toFlow(garden);
    expect(nodes.map((n) => n.id)).toEqual([
      "trigger-0",
      "condition-0",
      "condition-1",
      "action-0",
      "action-1",
    ]);
    expect(edges.map((e) => [e.source, e.target])).toEqual([
      ["trigger-0", "condition-0"],
      ["condition-0", "condition-1"],
      ["condition-1", "action-0"],
      ["action-0", "action-1"],
    ]);
  });

  it("lays a straight rule out top to bottom", () => {
    const ys = toFlow(garden).nodes.map((n) => n.position.y);
    expect([...ys].sort((a, b) => a - b)).toEqual(ys);
    expect(new Set(ys).size).toBe(ys.length);
  });

  it("joins the trigger straight to the actions without conditions", () => {
    const { edges } = toFlow({ ...garden, conditions: [] });
    expect(edges.map((e) => e.id)).toEqual(["trigger-0->action-0", "action-0->action-1"]);
  });

  it("puts the triggers side by side, each joined into what follows", () => {
    const { nodes } = toFlow(branching);
    const [first, second, condition] = nodes;
    expect(first.position.y).toBe(second.position.y);
    expect(first.position.x).not.toBe(second.position.x);
    expect(condition.position.y).toBeGreaterThan(first.position.y);
    expect(edgePairs(branching).slice(0, 2)).toEqual([
      ["trigger-0", "condition-0", null],
      ["trigger-1", "condition-0", null],
    ]);
  });

  it("names triggers by their place, and steps on the else side as such", () => {
    const headings = Object.fromEntries(toFlow(branching).nodes.map((n) => [n.id, n.data.heading]));
    expect(headings["trigger-0"]).toBe("Trigger 1");
    expect(headings["trigger-1"]).toBe("Trigger 2");
    expect(headings["condition-0"]).toBe("Only if");
    expect(headings["action-0.then-0"]).toBe("Then");
    expect(headings["action-0.else-0"]).toBe("Else");
  });

  it("splits an if into its branches, and joins them again at the next action", () => {
    expect(toFlow(branching).nodes.map((n) => n.id)).toEqual([
      "trigger-0",
      "trigger-1",
      "condition-0",
      "action-0",
      "action-0.then-0",
      "action-0.else-0",
      "action-1",
    ]);
    expect(edgePairs(branching).slice(2)).toEqual([
      ["condition-0", "action-0", null],
      ["action-0", "action-0.then-0", "then"],
      ["action-0", "action-0.else-0", "else"],
      ["action-0.then-0", "action-1", null],
      ["action-0.else-0", "action-1", null],
    ]);
  });

  it("goes straight past an empty branch, still saying which it is", () => {
    const rule: Draft = {
      ...branching,
      actions: [
        { ...(branching.actions[0] as Action & { do: "if" }), else: [] },
        branching.actions[1],
      ],
    };
    expect(edgePairs(rule)).toContainEqual(["action-0", "action-1", "else"]);
  });

  it("addresses steps inside branches inside branches", () => {
    expect(
      toFlow(nested)
        .nodes.map((n) => n.id)
        .slice(3),
    ).toEqual(["action-0", "action-0.then-0", "action-0.then-0.else-0", "action-0.then-0.else-1"]);
  });

  it("puts each step's summary on its node", () => {
    expect(toFlow(garden).nodes[0].data.summary).toBe("Office door opens");
  });
});

describe("summary", () => {
  const of = (step: Partial<Step> & Pick<Step, "role" | "value">, triggers: Trigger[] = []) =>
    summary({ id: "x", index: 0, branch: null, ...step } as Step, triggers);

  it("says what starts the rule", () => {
    expect(
      of({ role: "trigger", value: { type: "contact", device: "office/door", open: true } }),
    ).toBe("Office door opens");
    expect(
      of({ role: "trigger", value: { type: "contact", device: "office/door", open: false } }),
    ).toBe("Office door closes");
    expect(of({ role: "trigger", value: { type: "contact", device: "office/door" } })).toBe(
      "Office door opens or closes",
    );
    expect(
      of({ role: "trigger", value: { type: "device_state", room: "living_room", on: true } }),
    ).toBe("Something in Living room turns on");
    expect(
      of({
        role: "trigger",
        value: { type: "device_action", device: "office/go", action: "on_press" },
      }),
    ).toBe("Office go: on press");
    expect(
      of({
        role: "trigger",
        value: { type: "device_action", device: "x_bambulab:office/printer", action: "finished" },
      }),
    ).toBe("Office printer reports finished");
    expect(
      of({ role: "trigger", value: { type: "device_action", device: "x_bambulab:office/printer" } }),
    ).toBe("Office printer reports anything");
    expect(
      of({ role: "trigger", value: { type: "device_action", device: "office/wall_switch" } }),
    ).toBe("Office wall switch is pressed");
    expect(of({ role: "trigger", value: { type: "timer_fired" } })).toBe("Any timer goes off");
    expect(of({ role: "trigger", value: { type: "presence", state: "home" } })).toBe(
      "Somebody comes home",
    );
  });

  it("says how long a state has to hold", () => {
    expect(
      of({
        role: "trigger",
        value: { type: "contact", device: "office/door", open: true, for: { minutes: 10 } },
      }),
    ).toBe("Office door has been open for 10 minutes");
    expect(
      of({
        role: "trigger",
        value: {
          type: "device_state",
          device: "kitchen/kettle",
          on: true,
          for: { hours: 1, minutes: 30 },
        },
      }),
    ).toBe("Kitchen kettle has been on for 1 hour 30 minutes");
    expect(
      of({ role: "trigger", value: { type: "presence", state: "away", for: { hours: 2 } } }),
    ).toBe("Everybody has been away for 2 hours");
  });

  it("says readings, times and the sun", () => {
    expect(
      of({
        role: "trigger",
        value: { type: "numeric", device: "office", measure: "temperature", above: 24 },
      }),
    ).toBe("Office temperature above 24°");
    expect(
      of({
        role: "trigger",
        value: {
          type: "numeric",
          device: "office/sensor",
          measure: "humidity",
          above: 40,
          below: 60,
          for: { minutes: 5 },
        },
      }),
    ).toBe("Office sensor humidity has been between 40% and 60% for 5 minutes");
    expect(
      of({
        role: "trigger",
        value: {
          type: "numeric",
          device: "system/electricity",
          measure: "price",
          below: 1,
          for: { minutes: 15 },
        },
      }),
    ).toBe("System electricity price has been below 1 for 15 minutes");
    expect(
      of({
        role: "condition",
        value: { kind: "numeric", device: "office/air", measure: "co2", above: 1000 },
      }),
    ).toBe("Office air CO2 is above 1000");
    expect(
      of({
        role: "trigger",
        value: { type: "time", at: "22:00", days: ["mon", "tue", "wed", "thu", "fri"] },
      }),
    ).toBe("At 22:00 on weekdays");
    expect(of({ role: "trigger", value: { type: "time", at: "07:30" } })).toBe("At 07:30");
    expect(
      of({ role: "trigger", value: { type: "sun", event: "sunset", offset_minutes: -30 } }),
    ).toBe("30 minutes before sunset");
    expect(of({ role: "trigger", value: { type: "sun", event: "sunrise" } })).toBe("At sunrise");
  });

  it("says what has to hold", () => {
    expect(
      of({
        role: "condition",
        value: { kind: "time_of_day", after: "07:00", before: "23:00" },
      }),
    ).toBe("Between 07:00 and 23:00");
    expect(of({ role: "condition", value: { kind: "time_of_day", after: "22:00" } })).toBe(
      "After 22:00",
    );
    expect(of({ role: "condition", value: { kind: "time_of_day", days: ["sat", "sun"] } })).toBe(
      "At weekends",
    );
    expect(
      of({
        role: "condition",
        value: { kind: "contact_is", device: "hall/front_door", open: false },
      }),
    ).toBe("Hall front door is closed");
    expect(
      of({
        role: "condition",
        value: {
          kind: "sun",
          after: "sunset",
          before: "sunrise",
          before_offset_minutes: 60,
        },
      }),
    ).toBe("Between sunset and 1 hour after sunrise");
    expect(
      of({
        role: "condition",
        value: {
          kind: "numeric",
          device: "office/sensor",
          measure: "temperature",
          below: 18,
        },
      }),
    ).toBe("Office sensor temperature is below 18°");
    expect(of({ role: "condition", value: { kind: "presence_is", state: "away" } })).toBe(
      "Everybody is away",
    );
  });

  it("names a trigger by its place, never its id", () => {
    const by = (ids: string[]) =>
      of({ role: "condition", value: { kind: "triggered_by", triggers: ids } }, branching.triggers);
    expect(by(["opened"])).toBe("Only if started by Trigger 1");
    expect(by(["opened", "left-open"])).toBe("Only if started by Trigger 1 or Trigger 2");
    expect(by(["gone"])).toBe("Only if started by a trigger since removed");
  });

  it("says what happens", () => {
    expect(of({ role: "action", value: { do: "set_room", room: "office", on: false } })).toBe(
      "Turn off Office lights",
    );
    expect(
      of({
        role: "action",
        value: {
          do: "set_device",
          device: "living_room/lamp",
          on: true,
          brightness: 40,
          kelvin: 2700,
        },
      }),
    ).toBe("Turn on Living room lamp at 40%, 2700 K");
    expect(of({ role: "action", value: { do: "notify", body: "Door!", room: "office" } })).toBe(
      'Say "Door!" in Office',
    );
    expect(of({ role: "action", value: { do: "delay", minutes: 5 } })).toBe("Wait 5 minutes");
  });

  it("says an if as the choice it is", () => {
    expect(of({ role: "action", value: branching.actions[0] }, branching.triggers)).toBe(
      'If started by Trigger 1 then turn off Office lights else say "The garden door has been open for ten minutes"',
    );
    expect(
      of({
        role: "action",
        value: {
          do: "if",
          conditions: [{ kind: "contact_is", device: "office/door", open: true }],
          then: [],
        },
      }),
    ).toBe("If office door is open then do nothing");
  });

  it("says what is missing rather than nothing", () => {
    expect(of({ role: "trigger", value: { type: "contact", device: "", open: true } })).toBe(
      "A door or window opens",
    );
    expect(of({ role: "action", value: { do: "notify", body: "" } })).toBe("Say something");
  });
});

describe("days", () => {
  it("names the usual sets, and lists the rest", () => {
    expect(daysText(undefined)).toBeNull();
    expect(daysText(["mon", "tue", "wed", "thu", "fri", "sat", "sun"])).toBeNull();
    expect(daysText(["sat", "sun"])).toBe("at weekends");
    expect(daysText(["fri", "mon"])).toBe("on Mon and Fri");
    expect(daysText(["mon", "wed", "fri"])).toBe("on Mon, Wed and Fri");
  });

  it("toggles a day, keeping the week's order, and none is every day", () => {
    expect(toggleDay(["fri"], "mon")).toEqual(["mon", "fri"]);
    expect(toggleDay(["mon", "fri"], "fri")).toEqual(["mon"]);
    expect(toggleDay(["mon"], "mon")).toBeUndefined();
  });
});

describe("deviceLabel", () => {
  it("says a device the way it is named", () => {
    expect(deviceLabel("office/desk_lamp")).toBe("Office desk lamp");
    expect(deviceLabel("living_room/tv")).toBe("Living room tv");
    expect(deviceLabel("x_bambulab:office/printer")).toBe("Office printer");
  });
});

describe("deviceChoices", () => {
  const values = (kind: Parameters<typeof deviceChoices>[1], current?: string) =>
    deviceChoices(devices, kind, current).map((c) => c.value);

  it("offers doors and windows for contacts, a reported `open` included", () => {
    expect(values("contact")).toEqual(["hall/front_door", "office/door"]);
    expect(values("contact_is")).toEqual(["hall/front_door", "office/door"]);
  });

  it("offers what switches for device steps, without the source", () => {
    expect(values("set_device")).toEqual([
      "kitchen/kettle",
      "living_room/lamp",
      "wled:living_room/tv",
      "office/desk_lamp",
    ]);
  });

  it("offers switches and whatever has reported an action for an action", () => {
    const house = [
      ...devices,
      device("z2m:office/wall_switch", "switch"),
      { ...device("x_bambulab:office/printer", "sensor"), actions: true },
      device("x_bambulab:office/nozzle", "sensor", { temperature_celsius: 220 }),
    ];
    expect(deviceChoices(house, "device_action").map((c) => c.value)).toEqual([
      "x_bambulab:office/printer",
      "office/wall_switch",
    ]);
  });

  it("offers what reports the reading for a numeric step", () => {
    expect(deviceChoices(devices, "numeric", undefined, "temperature").map((c) => c.value)).toEqual(
      ["office/sensor"],
    );
    expect(deviceChoices(devices, "numeric", undefined, "brightness").map((c) => c.value)).toEqual([
      "living_room/lamp",
      "wled:living_room/tv",
      "office/desk_lamp",
    ]);
  });

  it("keeps the one a rule names even when nothing registered matches", () => {
    expect(values("contact", "garage/door")).toContain("garage/door");
  });

  it("labels each by room and name", () => {
    expect(deviceChoices(devices, "contact")[1]).toEqual({
      value: "office/door",
      label: "Office door",
    });
  });
});

describe("motion", () => {
  const of = (step: Partial<Step> & Pick<Step, "role" | "value">) =>
    summary({ id: "x", index: 0, branch: null, ...step } as Step);

  it("offers only motion sensors for a motion step", () => {
    const motion = deviceChoices(devices, "motion").map((c) => c.value);
    expect(motion).toEqual(["hall/motion"]);
    expect(deviceChoices(devices, "motion_is").map((c) => c.value)).toEqual(motion);
  });

  it("offers a sensor that reports motion before Niles has classified it", () => {
    const radar = device("z2m:office/radar", "unknown", { motion: true });
    expect(deviceChoices([radar], "motion").map((c) => c.value)).toEqual(["office/radar"]);
  });

  it("does not take a motion sensor for a button", () => {
    expect(deviceChoices(devices, "device_action").map((c) => c.value)).not.toContain(
      "hall/motion",
    );
  });

  it("offers a motion sensor's light level, and anything else that reports one", () => {
    const lux = (list: Device[]) =>
      deviceChoices(list, "numeric", undefined, "illuminance").map((c) => c.value);
    expect(lux(devices)).toEqual(["hall/motion"]);
    expect(lux([device("z2m:hall/motion", "motion")])).toEqual(["hall/motion"]);
    expect(lux([device("z2m:garden/lux", "sensor", { illuminance_lux: 400 })])).toEqual([
      "garden/lux",
    ]);
  });

  it("says what dim is under the light level", () => {
    expect(MEASURE_HINT.illuminance).toBe("Below about 30 is dim.");
    expect(MEASURE_HINT.temperature).toBeUndefined();
  });

  it("builds a motion trigger and a motion condition from the house", () => {
    expect(defaultTrigger("motion", devices)).toEqual({
      type: "motion",
      device: "hall/motion",
      detected: true,
    });
    expect(defaultCondition("motion_is", devices)).toEqual({
      kind: "motion_is",
      device: "hall/motion",
      detected: true,
    });
  });

  it("reads a motion trigger without how long", () => {
    const trigger = (detected?: boolean) =>
      of({ role: "trigger", value: { type: "motion", device: "hallway/motion", detected } });
    expect(trigger(true)).toBe("Hallway motion: sees motion");
    expect(trigger(false)).toBe("Hallway motion: motion stops");
    expect(trigger()).toBe("Hallway motion: motion starts or stops");
  });

  it("reads a motion trigger with how long", () => {
    const held = (detected?: boolean) =>
      of({
        role: "trigger",
        value: { type: "motion", device: "hallway/motion", detected, for: { minutes: 2 } },
      });
    expect(held(false)).toBe("Hallway motion: no motion for 2 minutes");
    expect(held(true)).toBe("Hallway motion: motion for 2 minutes");
    expect(held()).toBe("Hallway motion: unchanged for 2 minutes");
  });

  it("reads the motion condition and the light level", () => {
    expect(
      of({
        role: "condition",
        value: { kind: "motion_is", device: "hallway/motion", detected: true },
      }),
    ).toBe("Hallway motion sees motion");
    expect(
      of({
        role: "condition",
        value: { kind: "motion_is", device: "hallway/motion", detected: false },
      }),
    ).toBe("Hallway motion sees no motion");
    expect(
      of({
        role: "condition",
        value: { kind: "numeric", device: "hallway/motion", measure: "illuminance", below: 30 },
      }),
    ).toBe("Hallway motion light level is below 30 lux");
  });

  it("wants a motion sensor picked", () => {
    expect(
      problems({
        ...garden,
        triggers: [{ id: "t1", type: "motion", device: "", detected: false, for: { minutes: 2 } }],
        conditions: [{ kind: "motion_is", device: "", detected: true }],
      }),
    ).toEqual(["Trigger 1 needs a device.", "Condition 1 needs a device."]);
  });
});

describe("several motion sensors as one", () => {
  const of = (step: Partial<Step> & Pick<Step, "role" | "value">) =>
    summary({ id: "x", index: 0, branch: null, ...step } as Step);
  const both = ["hallway/motion", "kitchen/motion"];

  it("reads none of them seeing anybody as all of them, and motion as any", () => {
    const held = (detected: boolean) =>
      of({
        role: "trigger",
        value: { type: "motion", devices: both, detected, for: { minutes: 2 } },
      });
    expect(held(false)).toBe("Hallway motion and Kitchen motion: no motion for 2 minutes");
    expect(held(true)).toBe("Hallway motion or Kitchen motion: motion for 2 minutes");
    const named = { devices: ["hallway/motion"], rooms: ["kitchen"] };
    expect(of({ role: "trigger", value: { type: "motion", ...named, detected: true } })).toBe(
      "Hallway motion or every sensor in Kitchen: sees motion",
    );
    expect(
      of({ role: "condition", value: { kind: "motion_is", devices: both, detected: false } }),
    ).toBe("Hallway motion and Kitchen motion see no motion");
    expect(
      of({ role: "condition", value: { kind: "motion_is", devices: both, detected: true } }),
    ).toBe("Hallway motion or Kitchen motion sees motion");
  });

  it("goes from one sensor to several and back", () => {
    const one: Trigger = { type: "motion", device: "hall/motion", for: { minutes: 2 } };
    const several = withSeveralSensors(one, true, devices);
    // "Either" means nothing for a group: it waits for none.
    expect(several).toEqual({
      type: "motion",
      devices: ["hall/motion"],
      detected: false,
      for: { minutes: 2 },
    });
    expect(watchesSeveral(several)).toBe(true);
    expect(withSeveralSensors(several, false, devices)).toEqual({
      type: "motion",
      device: "hall/motion",
      detected: false,
      for: { minutes: 2 },
    });
    // Every box unticked is still several, asking for a sensor.
    const none = { ...several, devices: [] };
    expect(watchesSeveral(none)).toBe(true);
    expect(withSeveralSensors(none, false, devices)).toMatchObject({ device: "hall/motion" });
  });

  it("offers the motion sensors and the rooms with one", () => {
    const house = [...devices, device("z2m:kitchen/radar", "unknown", { motion: false })];
    const offered = sensorChoices(house, {
      kind: "motion_is",
      devices: ["attic/motion"],
      detected: false,
    });
    expect(offered.sensors.map((c) => c.value)).toEqual([
      "attic/motion",
      "hall/motion",
      "kitchen/radar",
    ]);
    expect(offered.rooms.map((c) => c.value)).toEqual(["hall", "kitchen"]);
  });

  it("wants a sensor, and whether it waits for motion or for none", () => {
    expect(
      problems({
        ...garden,
        triggers: [{ id: "t1", type: "motion", devices: [], rooms: [], for: { minutes: 2 } }],
        conditions: [{ kind: "motion_is", devices: [], detected: false }],
      }),
    ).toEqual([
      "Trigger 1 needs a device.",
      "Trigger 1: several sensors wait for motion in any, or for none in all.",
      "Condition 1 needs a device.",
    ]);
    expect(
      problems({
        ...garden,
        triggers: [{ id: "t1", type: "motion", rooms: ["kitchen"], detected: false }],
        conditions: [],
      }),
    ).toEqual([]);
  });
});

describe("blinds", () => {
  const of = (value: Action) => summary({ role: "action", id: "x", index: 0, branch: null, value });
  // The house above, with a blind in the office and a bedroom that has
  // nothing but blinds.
  const house = [
    ...devices,
    device("z2m:office/blind", "cover", { position: 0 }),
    device("z2m:bedroom/left_blind", "cover", { position: 100 }),
    device("z2m:bedroom/right_blind", "cover", { position: 40 }),
  ];

  it("reads like the server's sentence", () => {
    expect(of({ do: "set_blinds", room: "living_room", position: 0 })).toBe(
      "Close the living room blinds",
    );
    expect(of({ do: "set_blinds", room: "living_room", position: "open" })).toBe(
      "Open the living room blinds",
    );
    // A rule saved before this existed stores 100 and still reads as
    // Fully open.
    expect(of({ do: "set_blinds", room: "living_room", position: 100 })).toBe(
      "Fully open the living room blinds",
    );
    expect(of({ do: "set_blinds", device: "office/blind", position: 40 })).toBe(
      "Open the office blind to 40%",
    );
  });

  it("starts on a room with blinds, closing them", () => {
    expect(defaultAction("set_blinds", house)).toEqual({
      do: "set_blinds",
      room: "bedroom",
      position: 0,
    });
  });

  it("offers only rooms with a blind, and only blinds", () => {
    expect(roomChoices(house, undefined, "set_blinds").map((c) => c.value)).toEqual([
      "bedroom",
      "office",
    ]);
    expect(deviceChoices(house, "set_blinds").map((c) => c.value)).toEqual([
      "bedroom/left_blind",
      "bedroom/right_blind",
      "office/blind",
    ]);
  });

  it("is not offered by the light actions, nor taken for a button", () => {
    // A room is not one with lights because it has blinds: the server
    // never counts a blind as a light.
    expect(roomChoices(house, undefined, "set_room").map((c) => c.value)).not.toContain(
      "bedroom",
    );
    for (const kind of ["set_device", "device_state", "device_is", "device_action"] as const) {
      expect(deviceChoices(house, kind).map((c) => c.value)).not.toContain("office/blind");
    }
  });

  it("switches between a room and one blind, keeping the position", () => {
    const room: Action = { do: "set_blinds", room: "bedroom", position: 40 };
    expect(blindsTarget(room)).toBe("room");
    const one = withBlindsTarget(room, "device", house);
    expect(one).toEqual({ do: "set_blinds", device: "bedroom/left_blind", position: 40 });
    expect(blindsTarget(one)).toBe("device");
    expect(withBlindsTarget(one, "room", house)).toEqual({
      do: "set_blinds",
      room: "bedroom",
      position: 40,
    });
  });

  it("reads a position as close, open, fully or part of the way", () => {
    expect(blindsPosition(0)).toBe("close");
    expect(blindsPosition("open")).toBe("open");
    expect(blindsPosition(100)).toBe("fully");
    expect(blindsPosition(40)).toBe("part");
    expect(BLINDS_POSITION).toEqual({ close: 0, open: "open", fully: 100, part: 50 });
  });

  it("wants one target and a position the server takes", () => {
    const rule = (action: Action): Draft => ({ ...garden, actions: [action] });
    expect(problems(rule({ do: "set_blinds", room: "bedroom", position: 0 }))).toEqual([]);
    expect(problems(rule({ do: "set_blinds", device: "office/blind", position: 40 }))).toEqual(
      [],
    );
    expect(problems(rule({ do: "set_blinds", room: "", position: 0 }))).toEqual([
      "Action 1 needs a room or a blind.",
    ]);
    expect(
      problems(rule({ do: "set_blinds", room: "bedroom", device: "office/blind", position: 0 })),
    ).toEqual(["Action 1 names a room and a blind; it takes one or the other."]);
    expect(problems(rule({ do: "set_blinds", room: "bedroom", position: "open" }))).toEqual(
      [],
    );
    expect(problems(rule({ do: "set_blinds", room: "bedroom", position: 150 }))).toEqual([
      "Action 1: a blind opens 0 to 100%.",
    ]);
  });
});

describe("wake-up", () => {
  const of = (value: Action) => summary({ role: "action", id: "x", index: 0, branch: null, value });
  const rule = (action: Action): Draft => ({ ...garden, actions: [action] });

  it("reads like the server's sentence", () => {
    expect(of({ do: "wake_up" })).toBe("Wake the house up over 45 minutes, to 100%");
    expect(
      of({
        do: "wake_up",
        minutes: 30,
        rooms: ["office"],
        except: ["office/desk_lamp"],
        brightness: 80,
        kelvin: 2700,
      }),
    ).toBe("Wake the office up over 30 minutes, to 80% at 2700 K, but not the office desk lamp");
    expect(of({ do: "wake_up", devices: ["office/desk_lamp", "living_room/lamp"] })).toBe(
      "Wake the office desk lamp and the living room lamp up over 45 minutes, to 100%",
    );
  });

  it("says a warmth of its own, ramped or level", () => {
    expect(of({ do: "wake_up", start_kelvin: 2200, kelvin: 4000 })).toBe(
      "Wake the house up over 45 minutes, to 100%, from 2200 K to 4000 K",
    );
    expect(of({ do: "wake_up", kelvin: 3000 })).toBe(
      "Wake the house up over 45 minutes, to 100% at 3000 K",
    );
  });

  it("starts as the routine's: the whole house, over 45 minutes, to full", () => {
    expect(defaultAction("wake_up", devices)).toEqual({
      do: "wake_up",
      minutes: 45,
      brightness: 100,
    });
  });

  it("switches between the house, rooms and lights, keeping the levels", () => {
    const house: Action = {
      do: "wake_up",
      minutes: 30,
      brightness: 80,
      except: ["office/desk_lamp"],
    };
    expect(wakeTarget(house)).toBe("house");
    const rooms = withWakeTarget(house, "rooms", devices);
    expect(rooms).toEqual({ do: "wake_up", minutes: 30, brightness: 80, rooms: ["living_room"] });
    const lights = withWakeTarget(rooms, "lights", devices);
    expect(lights).toEqual({
      do: "wake_up",
      minutes: 30,
      brightness: 80,
      devices: ["kitchen/kettle"],
    });
    expect(withWakeTarget(lights, "house", devices)).toEqual({
      do: "wake_up",
      minutes: 30,
      brightness: 80,
    });
  });

  it("stays on rooms with every room unticked, rather than becoming the house", () => {
    const none: Action = { do: "wake_up", rooms: [] };
    expect(wakeTarget(none)).toBe("rooms");
    expect(problems(rule(none))).toEqual(["Action 1 needs a room."]);
    expect(problems(rule({ do: "wake_up", devices: [] }))).toEqual(["Action 1 needs a light."]);
  });

  it("offers as exceptions only the lights in the rooms ticked", () => {
    const choices = wakeChoices(devices, { do: "wake_up", rooms: ["living_room"] });
    expect(choices.except.map((c) => c.value)).toEqual([
      "living_room/lamp",
      "wled:living_room/tv",
    ]);
    expect(choices.lights.map((c) => c.value)).toContain("office/desk_lamp");
    expect(choices.rooms.map((c) => c.value)).toContain("living_room");
  });

  it("keeps a light already ticked even when nothing registered matches it", () => {
    const gone = wakeChoices(devices, { do: "wake_up", except: ["attic/old_lamp"] });
    expect(gone.except.map((c) => c.value)).toContain("attic/old_lamp");
  });

  it("wants the bounds the server takes", () => {
    expect(problems(rule({ do: "wake_up", minutes: 30, brightness: 80 }))).toEqual([]);
    expect(problems(rule({ do: "wake_up", minutes: 0 }))).toEqual([
      "Action 1: a wake-up takes 1 to 120 minutes.",
    ]);
    expect(problems(rule({ do: "wake_up", minutes: 121 }))).toEqual([
      "Action 1: a wake-up takes 1 to 120 minutes.",
    ]);
    expect(problems(rule({ do: "wake_up", brightness: 0 }))).toEqual([
      "Action 1: a wake-up ends at 1 to 100%.",
    ]);
    expect(problems(rule({ do: "wake_up", kelvin: 9000 }))).toEqual([
      "Action 1: colour temperature is 2000 to 6500 K.",
    ]);
    expect(problems(rule({ do: "wake_up", start_kelvin: 2200 }))).toEqual([
      "Action 1: a start warmth needs an end warmth.",
    ]);
    expect(problems(rule({ do: "wake_up", start_kelvin: 1000, kelvin: 4000 }))).toEqual([
      "Action 1's start: colour temperature is 2000 to 6500 K.",
    ]);
  });
});

describe("roomChoices", () => {
  it("offers every room once, by name", () => {
    expect(roomChoices(devices)).toEqual([
      { value: "hall", label: "Hall" },
      { value: "kitchen", label: "Kitchen" },
      { value: "living_room", label: "Living room" },
      { value: "office", label: "Office" },
    ]);
  });

  it("keeps a room nothing is in any more", () => {
    expect(roomChoices([], "attic")).toEqual([{ value: "attic", label: "Attic" }]);
  });
});

describe("measureChoices", () => {
  const fixed = ["Temperature", "Humidity", "Battery", "Brightness", "Light level (lux)"];

  it("offers every measure a rule can watch", () => {
    expect(measureChoices(devices).map((c) => c.label)).toEqual(fixed);
  });

  it("adds the readings the house reports, labelled with their units", () => {
    const house = [
      ...devices,
      device("z2m:system/electricity", "sensor", {
        readings: { price: { value: 1.23, unit: "kr/kWh" } },
      }),
      device("z2m:kitchen/dishwasher", "outlet", {
        readings: {
          power: { value: 340, unit: "W" },
          energy: { value: 12.4, unit: "kWh" },
        },
      }),
      device("z2m:office/air", "sensor", {
        readings: { co2: { value: 812, unit: "ppm" }, ratio: { value: 0.5, unit: null } },
      }),
    ];
    expect(measureChoices(house)).toEqual([
      ...fixed.map((label) => expect.objectContaining({ label })),
      { value: "co2", label: "CO2 (ppm)" },
      { value: "energy", label: "Energy (kWh)" },
      { value: "power", label: "Power (W)" },
      { value: "price", label: "Price (kr/kWh)" },
      { value: "ratio", label: "Ratio" },
    ]);
  });

  it("keeps a reading nothing reports any more", () => {
    expect(measureChoices(devices, "price").at(-1)).toEqual({ value: "price", label: "Price" });
    expect(measureChoices(devices, "temperature")).toHaveLength(fixed.length);
  });

  it("offers the devices that report the reading chosen", () => {
    const house = [
      ...devices,
      device("z2m:kitchen/dishwasher", "outlet", {
        readings: { power: { value: 340, unit: "W" } },
      }),
      device("z2m:office/air", "sensor", { readings: { co2: { value: 812, unit: "ppm" } } }),
    ];
    const reporting = (measure: string) =>
      deviceChoices(house, "numeric", undefined, measure).map((c) => c.value);
    expect(reporting("power")).toEqual(["kitchen/dishwasher"]);
    expect(reporting("co2")).toEqual(["office/air"]);
    expect(deviceChoices(house, "numeric").map((c) => c.value)).toContain("office/air");
  });
});

describe("triggerChoices", () => {
  it("offers each trigger by place and sentence, keyed by its id", () => {
    expect(triggerChoices(branching.triggers)).toEqual([
      { value: "opened", label: "Trigger 1 — Office door opens" },
      { value: "left-open", label: "Trigger 2 — Office door has been open for 10 minutes" },
    ]);
  });
});

describe("newRule", () => {
  it("starts as the door turning its room's lights off", () => {
    expect(newRule(devices)).toEqual({
      description: "",
      enabled: true,
      triggers: [{ id: "t1", type: "contact", device: "office/door", open: true }],
      conditions: [],
      actions: [{ do: "set_room", room: "office", on: false }],
    });
  });

  it("leaves the door to pick in a house without one", () => {
    const rule = newRule([]);
    expect(rule.triggers).toEqual([{ id: "t1", type: "contact", device: "", open: true }]);
    expect(problems(rule)).toContain("Trigger 1 needs a device.");
  });
});

describe("defaults", () => {
  it("fills each kind in from the house", () => {
    expect(defaultTrigger("presence", devices)).toEqual({ type: "presence", state: "home" });
    expect(defaultTrigger("numeric", devices)).toEqual({
      type: "numeric",
      device: "office/sensor",
      measure: "temperature",
      above: 24,
    });
    expect(defaultTrigger("time", devices)).toEqual({ type: "time", at: "07:00" });
    expect(defaultTrigger("sun", devices)).toEqual({ type: "sun", event: "sunset" });
    expect(defaultCondition("contact_is", devices)).toEqual({
      kind: "contact_is",
      device: "hall/front_door",
      open: false,
    });
    expect(defaultCondition("sun", devices)).toEqual({
      kind: "sun",
      after: "sunset",
      before: "sunrise",
    });
    expect(defaultAction("set_device", devices)).toEqual({
      do: "set_device",
      device: "kitchen/kettle",
      on: true,
    });
    expect(defaultAction("set_room", devices)).toEqual({
      do: "set_room",
      room: "hall",
      on: true,
    });
    expect(defaultAction("delay", devices)).toEqual({ do: "delay", minutes: 5 });
  });

  it("starts a which-trigger check, and an if with several triggers, on the first", () => {
    expect(defaultCondition("triggered_by", devices, branching.triggers)).toEqual({
      kind: "triggered_by",
      triggers: ["opened"],
    });
    expect(defaultAction("if", devices, branching.triggers)).toEqual({
      do: "if",
      conditions: [{ kind: "triggered_by", triggers: ["opened"] }],
      then: [],
    });
    expect(defaultAction("if", devices, garden.triggers)).toMatchObject({
      conditions: [{ kind: "time_of_day" }],
    });
  });
});

describe("move", () => {
  it("moves one entry by one place", () => {
    expect(move(["a", "b", "c"], 0, 1)).toEqual(["b", "a", "c"]);
    expect(move(["a", "b", "c"], 2, -1)).toEqual(["a", "c", "b"]);
  });

  it("goes no further than either end", () => {
    const list = ["a", "b"];
    expect(move(list, 0, -1)).toBe(list);
    expect(move(list, 1, 1)).toBe(list);
  });
});

describe("editing steps", () => {
  it("replaces one step", () => {
    const step = only(garden, "action-0");
    const next = withStep(garden, {
      ...step,
      value: { do: "set_room", room: "office", on: true },
    } as Step);
    expect(next.actions[0]).toEqual({ do: "set_room", room: "office", on: true });
    expect(next.actions[1]).toBe(garden.actions[1]);
  });

  it("removes conditions and actions, never the last trigger", () => {
    expect(withoutStep(garden, "condition-0").conditions).toEqual([garden.conditions[1]]);
    expect(withoutStep(garden, "action-1").actions).toEqual([garden.actions[0]]);
    expect(withoutStep(garden, "trigger-0")).toBe(garden);
  });

  it("moves a step and follows it", () => {
    const moved = moveStep(garden, "action-0", 1);
    expect(moved.id).toBe("action-1");
    expect(moved.rule.actions).toEqual([garden.actions[1], garden.actions[0]]);
    expect(moveStep(garden, "action-1", 1).id).toBe("action-1");
  });

  it("numbers steps in order", () => {
    expect(stepsOf(garden).map((s) => s.id)).toEqual([
      "trigger-0",
      "condition-0",
      "condition-1",
      "action-0",
      "action-1",
    ]);
  });
});

describe("editing steps inside branches", () => {
  it("finds a step by its path", () => {
    const step = only(nested, "action-0.then-0.else-1");
    expect(step).toMatchObject({ role: "action", index: 1, branch: "else" });
    expect(step.value).toEqual({ do: "set_room", room: "office", on: false });
    expect(stepAt(nested, "action-0.else-0")).toBeUndefined();
  });

  it("replaces a step deep down and nothing else", () => {
    const step = only(nested, "action-0.then-0.else-0");
    const next = withStep(nested, {
      ...step,
      value: { do: "delay", seconds: 30 },
    } as Step);
    expect(only(next, "action-0.then-0.else-0").value).toEqual({ do: "delay", seconds: 30 });
    expect(only(next, "action-0.then-0.else-1").value).toEqual(
      only(nested, "action-0.then-0.else-1").value,
    );
    expect(next.triggers).toBe(nested.triggers);
  });

  it("removes a step from its branch", () => {
    const next = withoutStep(nested, "action-0.then-0.else-0");
    expect(stepsOf(next).map((s) => s.id)).toContain("action-0.then-0.else-0");
    expect(stepsOf(next).map((s) => s.id)).not.toContain("action-0.then-0.else-1");
    expect(only(next, "action-0.then-0.else-0").value).toEqual({
      do: "set_room",
      room: "office",
      on: false,
    });
  });

  it("moves a step within its branch and follows it", () => {
    const moved = moveStep(nested, "action-0.then-0.else-0", 1);
    expect(moved.id).toBe("action-0.then-0.else-1");
    expect(only(moved.rule, "action-0.then-0.else-1").value).toEqual({
      do: "set_device",
      device: "office/desk_lamp",
      on: true,
    });
    expect(moveStep(nested, "action-0.then-0.else-1", 1).id).toBe("action-0.then-0.else-1");
  });

  it("counts a step's siblings in its own list", () => {
    expect(siblings(nested, only(nested, "action-0.then-0.else-1"))).toBe(2);
    expect(siblings(nested, only(nested, "action-0"))).toBe(1);
    expect(siblings(branching, only(branching, "trigger-1"))).toBe(2);
  });

  it("adds an action to either branch of an if, and says where", () => {
    const wait: Action = { do: "delay", minutes: 1 };
    const intoElse = addAction(nested, wait, { id: "action-0", branch: "else" });
    expect(intoElse.id).toBe("action-0.else-0");
    expect(only(intoElse.rule, "action-0.else-0").value).toEqual(wait);

    const intoThen = addAction(nested, wait, { id: "action-0.then-0", branch: "then" });
    expect(intoThen.id).toBe("action-0.then-0.then-0");
    expect(only(intoThen.rule, "action-0.then-0.then-0").value).toEqual(wait);

    const atEnd = addAction(garden, wait);
    expect(atEnd.id).toBe("action-2");
  });
});

describe("triggers", () => {
  it("gives a new trigger the next free id", () => {
    expect(
      nextTriggerId([
        { id: "t1", type: "timer_fired" },
        { id: "t3", type: "timer_fired" },
      ]),
    ).toBe("t2");
    const added = addTrigger(garden, { type: "presence", state: "away" });
    expect(added.id).toBe("trigger-1");
    expect(added.rule.triggers[1]).toEqual({ id: "t2", type: "presence", state: "away" });
  });

  it("fills in ids a rule came without", () => {
    const rule = withTriggerIds({
      ...garden,
      triggers: [{ type: "timer_fired" }, { id: "t1", type: "presence" }],
    });
    expect(rule.triggers.map((t) => t.id)).toEqual(["t2", "t1"]);
  });

  it("keeps a trigger's id when its kind changes", () => {
    const step = only(branching, "trigger-0");
    const next = withStep(branching, { ...step, value: { type: "time", at: "08:00" } } as Step);
    expect(next.triggers[0]).toEqual({ id: "opened", type: "time", at: "08:00" });
  });

  it("takes a removed trigger out of every triggered_by, however deep", () => {
    const rule: Draft = {
      ...nested,
      conditions: [{ kind: "triggered_by", triggers: ["opened", "left-open"] }],
    };
    const next = withoutStep(rule, "trigger-1");
    expect(next.triggers.map((t) => t.id)).toEqual(["opened"]);
    expect(next.conditions).toEqual([{ kind: "triggered_by", triggers: ["opened"] }]);
    expect(only(next, "action-0.then-0").value).toMatchObject({
      conditions: [{ kind: "triggered_by", triggers: [] }],
    });
    expect(problems(next)).toContain(
      "Action 1 › then 1 › condition 1 needs a trigger to have started it.",
    );
  });

  it("moves a trigger, its id going with it", () => {
    const moved = moveStep(branching, "trigger-1", -1);
    expect(moved.id).toBe("trigger-0");
    expect(moved.rule.triggers.map((t) => t.id)).toEqual(["left-open", "opened"]);
  });
});

describe("problems", () => {
  it("finds none in a whole rule", () => {
    expect(problems(garden)).toEqual([]);
    expect(problems(branching)).toEqual([]);
    expect(problems(nested)).toEqual([]);
  });

  it("wants an action", () => {
    expect(problems({ ...garden, actions: [] })).toEqual([
      "An automation needs at least one action.",
    ]);
  });

  it("wants a trigger, and no more than ten", () => {
    expect(problems({ ...garden, triggers: [] })).toEqual([
      "An automation needs at least one trigger.",
    ]);
    const many: Trigger[] = Array.from({ length: 11 }, (_, i) => ({
      id: `t${i + 1}`,
      type: "timer_fired",
    }));
    expect(problems({ ...garden, triggers: many })).toEqual([
      "An automation has at most 10 triggers.",
    ]);
  });

  it("wants every device and room picked", () => {
    expect(
      problems({
        ...garden,
        conditions: [{ kind: "device_is", device: "", on: true }],
        actions: [
          { do: "set_device", device: "", on: true },
          { do: "set_room", room: "", on: false },
        ],
      }),
    ).toEqual([
      "Condition 1 needs a device.",
      "Action 1 needs a device.",
      "Action 2 needs a room.",
    ]);
  });

  it("keeps levels in range", () => {
    expect(
      problems({
        ...garden,
        actions: [{ do: "set_room", room: "office", on: true, brightness: 140, kelvin: 1000 }],
      }),
    ).toEqual([
      "Action 1: brightness is 0 to 100%.",
      "Action 1: colour temperature is 2000 to 6500 K.",
    ]);
  });

  it("wants something to say and a time window with an edge", () => {
    expect(
      problems({
        ...garden,
        conditions: [{ kind: "time_of_day" }],
        actions: [{ do: "notify", body: " " }],
      }),
    ).toEqual(["Condition 1 needs a start or an end.", "Action 1 needs something to say."]);
  });

  it("keeps how long and how long to wait above nothing and within a day", () => {
    expect(
      problems({
        ...garden,
        triggers: [
          {
            id: "t1",
            type: "contact",
            device: "office/door",
            open: true,
            for: { hours: 25 },
          },
          { id: "t2", type: "presence", state: "home", for: { minutes: 0 } },
        ],
        actions: [{ do: "delay" }, { do: "delay", hours: 24 }],
      }),
    ).toEqual([
      "Trigger 1: how long is more than nothing and at most 24 hours.",
      "Trigger 2: how long is more than nothing and at most 24 hours.",
      "Action 1: a wait is more than nothing and at most 24 hours.",
    ]);
  });

  it("wants a reading's value and a time that is a time", () => {
    expect(
      problems({
        ...garden,
        triggers: [
          { id: "t1", type: "numeric", device: "office/sensor", measure: "temperature" },
          { id: "t2", type: "time", at: "7:00" },
        ],
        conditions: [{ kind: "sun" }],
      }),
    ).toEqual([
      "Trigger 1 needs a value to go above or below.",
      "Trigger 2: 7:00 is not a time.",
      "Condition 1 needs a sunrise or a sunset.",
    ]);
  });

  it("wants triggered_by to name triggers the rule has", () => {
    expect(
      problems({ ...branching, conditions: [{ kind: "triggered_by", triggers: ["gone"] }] }),
    ).toEqual(["Condition 1 names a trigger that is gone."]);
  });

  it("goes no more than three ifs deep", () => {
    const deep = (depth: number): Action =>
      depth === 0
        ? { do: "set_room", room: "office", on: false }
        : {
            do: "if",
            conditions: [{ kind: "presence_is", state: "home" }],
            then: [deep(depth - 1)],
          };
    expect(problems({ ...garden, actions: [deep(3)] })).toEqual([]);
    expect(problems({ ...garden, actions: [deep(4)] })).toEqual([
      "Action 1 › then 1 › then 1 › then 1: an if goes at most 3 deep.",
    ]);
  });

  it("names a step inside a branch by its path", () => {
    expect(
      problems({
        ...branching,
        actions: [
          {
            do: "if",
            conditions: [{ kind: "triggered_by", triggers: ["opened"] }],
            then: [],
            else: [{ do: "notify", body: "" }],
          },
        ],
      }),
    ).toEqual(["Action 1 › else 1 needs something to say."]);
  });
});

describe("runs", () => {
  const now = new Date(2026, 9, 12, 15, 0);
  const at = (day: number, hour: number, minute: number) =>
    new Date(2026, 9, day, hour, minute).toISOString();
  const run = (outcome: Run["outcome"], reason: string | null = null): Run => ({
    started_at: at(12, 14, 2),
    trigger: "left-open",
    outcome,
    reason,
    steps: [],
  });

  it("says when, the way it is said", () => {
    expect(ranAt(at(12, 14, 2), now)).toBe("Today 14:02");
    expect(ranAt(at(11, 22, 10), now)).toBe("Yesterday 22:10");
    expect(ranAt(at(9, 8, 5), now)).toBe("Fri 08:05");
    expect(ranAt(new Date(2026, 8, 30, 8, 30).toISOString(), now)).toBe("30 Sep 08:30");
  });

  it("names the trigger by its place", () => {
    expect(runTrigger(run("done"), branching.triggers)).toBe("Trigger 2");
    expect(runTrigger({ ...run("done"), trigger: null }, branching.triggers)).toBeNull();
    expect(runTrigger({ ...run("done"), trigger: "gone" }, branching.triggers)).toBeNull();
  });

  it("puts the last run in one line", () => {
    expect(lastRunText(run("done"), now)).toBe("Last run today 14:02 — done");
    expect(
      lastRunText({ ...run("stopped", "it was before 07:00"), started_at: at(11, 22, 10) }, now),
    ).toBe("Last run yesterday 22:10 — stopped: it was before 07:00");
  });
});
