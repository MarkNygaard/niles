import { describe, expect, it } from "vitest";
import {
  defaultAction,
  defaultCondition,
  defaultTrigger,
  deviceChoices,
  deviceLabel,
  move,
  moveStep,
  newRule,
  problems,
  roomChoices,
  stepAt,
  stepsOf,
  summary,
  toFlow,
  withStep,
  withoutStep,
} from "./automations";
import type { Draft, Step } from "./automations";
import type { Device, DeviceState } from "./api";

const quiet: DeviceState = {
  on: null,
  brightness: null,
  color_temp_kelvin: null,
  rgb: null,
  temperature_celsius: null,
  humidity_percent: null,
  battery_percent: null,
  open: null,
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
];

const garden: Draft = {
  description: "Garden door",
  enabled: true,
  trigger: { type: "contact", device: "office/door", open: true },
  conditions: [
    { kind: "time_of_day", after: "07:00", before: "23:00" },
    { kind: "device_is", device: "office/desk_lamp", on: true },
  ],
  actions: [
    { do: "set_room", room: "office", on: false },
    { do: "notify", body: "The garden door is open" },
  ],
};

const only = (rule: Draft, id: string) => stepAt(rule, id)!;

describe("toFlow", () => {
  it("is one line: the trigger, the conditions, then the actions", () => {
    const { nodes, edges } = toFlow(garden);
    expect(nodes.map((n) => n.id)).toEqual([
      "trigger",
      "condition-0",
      "condition-1",
      "action-0",
      "action-1",
    ]);
    expect(edges.map((e) => [e.source, e.target])).toEqual([
      ["trigger", "condition-0"],
      ["condition-0", "condition-1"],
      ["condition-1", "action-0"],
      ["action-0", "action-1"],
    ]);
  });

  it("lays it out top to bottom", () => {
    const ys = toFlow(garden).nodes.map((n) => n.position.y);
    expect([...ys].sort((a, b) => a - b)).toEqual(ys);
    expect(new Set(ys).size).toBe(ys.length);
  });

  it("joins the trigger straight to the actions without conditions", () => {
    const { edges } = toFlow({ ...garden, conditions: [] });
    expect(edges.map((e) => e.id)).toEqual(["trigger->action-0", "action-0->action-1"]);
  });

  it("puts each step's summary on its node", () => {
    expect(toFlow(garden).nodes[0].data.summary).toBe("Office door opens");
  });
});

describe("summary", () => {
  const of = (step: Partial<Step> & Pick<Step, "role" | "value">) =>
    summary({ id: "x", index: 0, ...step } as Step);

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
    expect(of({ role: "trigger", value: { type: "timer_fired" } })).toBe("Any timer goes off");
    expect(of({ role: "trigger", value: { type: "presence", state: "home" } })).toBe(
      "Somebody comes home",
    );
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
    expect(
      of({
        role: "condition",
        value: { kind: "contact_is", device: "hall/front_door", open: false },
      }),
    ).toBe("Hall front door is closed");
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
  });

  it("says what is missing rather than nothing", () => {
    expect(of({ role: "trigger", value: { type: "contact", device: "", open: true } })).toBe(
      "A door or window opens",
    );
    expect(of({ role: "action", value: { do: "notify", body: "" } })).toBe("Say something");
  });
});

describe("deviceLabel", () => {
  it("says a device the way it is named", () => {
    expect(deviceLabel("office/desk_lamp")).toBe("Office desk lamp");
    expect(deviceLabel("living_room/tv")).toBe("Living room tv");
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
      "living_room/tv",
      "office/desk_lamp",
    ]);
  });

  it("offers what is left for a button", () => {
    expect(values("device_action")).toEqual(["office/go"]);
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

describe("newRule", () => {
  it("starts as the door turning its room's lights off", () => {
    expect(newRule(devices)).toEqual({
      description: "",
      enabled: true,
      trigger: { type: "contact", device: "office/door", open: true },
      conditions: [],
      actions: [{ do: "set_room", room: "office", on: false }],
    });
  });

  it("leaves the door to pick in a house without one", () => {
    const rule = newRule([]);
    expect(rule.trigger).toEqual({ type: "contact", device: "", open: true });
    expect(problems(rule)).toContain("The trigger needs a device.");
  });
});

describe("defaults", () => {
  it("fills each kind in from the house", () => {
    expect(defaultTrigger("presence", devices)).toEqual({ type: "presence", state: "home" });
    expect(defaultCondition("contact_is", devices)).toEqual({
      kind: "contact_is",
      device: "hall/front_door",
      open: false,
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

  it("removes conditions and actions, never the trigger", () => {
    expect(withoutStep(garden, "condition-0").conditions).toEqual([garden.conditions[1]]);
    expect(withoutStep(garden, "action-1").actions).toEqual([garden.actions[0]]);
    expect(withoutStep(garden, "trigger")).toBe(garden);
  });

  it("moves a step and follows it", () => {
    const moved = moveStep(garden, "action-0", 1);
    expect(moved.id).toBe("action-1");
    expect(moved.rule.actions).toEqual([garden.actions[1], garden.actions[0]]);
    expect(moveStep(garden, "action-1", 1).id).toBe("action-1");
  });

  it("numbers steps in order", () => {
    expect(stepsOf(garden).map((s) => s.id)).toEqual([
      "trigger",
      "condition-0",
      "condition-1",
      "action-0",
      "action-1",
    ]);
  });
});

describe("problems", () => {
  it("finds none in a whole rule", () => {
    expect(problems(garden)).toEqual([]);
  });

  it("wants an action", () => {
    expect(problems({ ...garden, actions: [] })).toEqual([
      "An automation needs at least one action.",
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
});
