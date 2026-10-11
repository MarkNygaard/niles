import dagre from "@dagrejs/dagre";
import { MarkerType } from "@xyflow/react";
import type { Edge, Node } from "@xyflow/react";
import type { Action, Condition, Device, Rule, Trigger } from "@/lib/api";
import { humanize } from "@/lib/rooms";

/**
 * Rules as the Automations page draws and edits them.
 *
 * A rule is one straight line: its trigger, then its conditions, then
 * its actions. The flow is a picture of that line and nothing more, so
 * edges and positions are worked out from the order every time rather
 * than stored anywhere.
 */

export type TriggerKind = Trigger["type"];
export type ConditionKind = Condition["kind"];
export type ActionKind = Action["do"];
export type StepKind = TriggerKind | ConditionKind | ActionKind;

/** One node of the line, and where in the rule it lives. */
export type Step =
  | { role: "trigger"; id: "trigger"; value: Trigger }
  | { role: "condition"; id: string; index: number; value: Condition }
  | { role: "action"; id: string; index: number; value: Action };

export const TRIGGER_KINDS: TriggerKind[] = [
  "contact",
  "device_state",
  "device_action",
  "timer_fired",
  "presence",
];
export const CONDITION_KINDS: ConditionKind[] = ["time_of_day", "device_is", "contact_is"];
export const ACTION_KINDS: ActionKind[] = ["set_room", "set_device", "notify"];

/** What each kind is called on a node, in the palette and in the
    trigger picker. */
export const KIND_LABEL: Record<StepKind, string> = {
  contact: "Door or window",
  device_state: "Device turns on or off",
  device_action: "Button press",
  timer_fired: "Timer goes off",
  presence: "Home or away",
  time_of_day: "Time of day",
  device_is: "Device is on or off",
  contact_is: "Door or window is",
  set_room: "Lights in a room",
  set_device: "One device",
  notify: "Say something",
};

export const ROLE_LABEL: Record<Step["role"], string> = {
  trigger: "When",
  condition: "Only if",
  action: "Then",
};

export function kindOf(step: Step): StepKind {
  switch (step.role) {
    case "trigger":
      return step.value.type;
    case "condition":
      return step.value.kind;
    case "action":
      return step.value.do;
  }
}

export function stepsOf(rule: Pick<Rule, "trigger" | "conditions" | "actions">): Step[] {
  return [
    { role: "trigger", id: "trigger", value: rule.trigger },
    ...rule.conditions.map((value, index): Step => ({
      role: "condition",
      id: `condition-${index}`,
      index,
      value,
    })),
    ...rule.actions.map((value, index): Step => ({
      role: "action",
      id: `action-${index}`,
      index,
      value,
    })),
  ];
}

export function stepAt<R extends Pick<Rule, "trigger" | "conditions" | "actions">>(
  rule: R,
  id: string | null | undefined,
): Step | undefined {
  return stepsOf(rule).find((step) => step.id === id);
}

/** A rule being made has no id yet; everything else is the same. */
export type Draft = Omit<Rule, "id"> & { id?: string };

/** The rule with one step's value replaced. */
export function withStep<R extends Draft>(rule: R, step: Step): R {
  switch (step.role) {
    case "trigger":
      return { ...rule, trigger: step.value };
    case "condition":
      return {
        ...rule,
        conditions: rule.conditions.map((c, i) => (i === step.index ? step.value : c)),
      };
    case "action":
      return {
        ...rule,
        actions: rule.actions.map((a, i) => (i === step.index ? step.value : a)),
      };
  }
}

/** The rule without a condition or an action. The trigger stays: a
    rule has exactly one. */
export function withoutStep<R extends Draft>(rule: R, id: string): R {
  const step = stepAt(rule, id);
  if (step?.role === "condition") {
    return { ...rule, conditions: rule.conditions.filter((_, i) => i !== step.index) };
  }
  if (step?.role === "action") {
    return { ...rule, actions: rule.actions.filter((_, i) => i !== step.index) };
  }
  return rule;
}

/** `list` with the entry at `i` moved `by` places, or as it was when
    that would go past either end. */
export function move<T>(list: T[], i: number, by: number): T[] {
  const to = i + by;
  if (i < 0 || i >= list.length || to < 0 || to >= list.length || by === 0) return list;
  const next = [...list];
  const [entry] = next.splice(i, 1);
  next.splice(to, 0, entry);
  return next;
}

/** Move a condition or an action along its part of the line, and the
    id it has afterwards, so it can stay selected. */
export function moveStep<R extends Draft>(
  rule: R,
  id: string,
  by: number,
): { rule: R; id: string } {
  const step = stepAt(rule, id);
  if (step?.role === "condition") {
    const conditions = move(rule.conditions, step.index, by);
    const at = conditions === rule.conditions ? step.index : step.index + by;
    return { rule: { ...rule, conditions }, id: `condition-${at}` };
  }
  if (step?.role === "action") {
    const actions = move(rule.actions, step.index, by);
    const at = actions === rule.actions ? step.index : step.index + by;
    return { rule: { ...rule, actions }, id: `action-${at}` };
  }
  return { rule, id };
}

/** `office/door` → "Office door": a device the way it is said. */
export function deviceLabel(device: string): string {
  const slash = device.indexOf("/");
  if (slash === -1) return humanize(device);
  const room = device.slice(0, slash);
  const name = device.slice(slash + 1).replace(/_/g, " ");
  return `${humanize(room)} ${name}`;
}

/** `z2m:office/door` → `office/door`: rules name devices without the
    source, which Niles fills in. */
export function ruleDeviceName(device: Device): string {
  return device.id.slice(device.id.indexOf(":") + 1);
}

function device(name: string | undefined, missing: string): string {
  return name ? deviceLabel(name) : missing;
}

function setting(target: string, s: { on?: boolean; brightness?: number; kelvin?: number }) {
  if (s.on === false) return `Turn off ${target}`;
  const levels = [
    s.brightness !== undefined && `${s.brightness}%`,
    s.kelvin !== undefined && `${s.kelvin} K`,
  ].filter(Boolean);
  const verb = s.on === true ? `Turn on ${target}` : `Set ${target}`;
  return levels.length > 0 ? `${verb} at ${levels.join(", ")}` : verb;
}

/** One line for a node: what this step does, in a few words. */
export function summary(step: Step): string {
  if (step.role === "trigger") {
    const t = step.value;
    switch (t.type) {
      case "contact": {
        const what = device(t.device, "A door or window");
        if (t.open === undefined) return `${what} opens or closes`;
        return `${what} ${t.open ? "opens" : "closes"}`;
      }
      case "device_state": {
        const what = t.device
          ? deviceLabel(t.device)
          : t.room
            ? `Something in ${humanize(t.room)}`
            : "Any device";
        return `${what} ${t.on === undefined ? "turns on or off" : t.on ? "turns on" : "turns off"}`;
      }
      case "device_action": {
        const what = device(t.device, "A button");
        return t.action ? `${what}: ${t.action.replace(/_/g, " ")}` : `${what} is pressed`;
      }
      case "timer_fired":
        return t.name ? `The ${t.name} timer goes off` : "Any timer goes off";
      case "presence":
        return t.state === "home"
          ? "Somebody comes home"
          : t.state === "away"
            ? "Everybody leaves"
            : t.state === "unknown"
              ? "Nobody can tell who is home"
              : "Home or away changes";
    }
  }
  if (step.role === "condition") {
    const c = step.value;
    switch (c.kind) {
      case "time_of_day":
        if (c.after && c.before) return `Between ${c.after} and ${c.before}`;
        if (c.after) return `After ${c.after}`;
        if (c.before) return `Before ${c.before}`;
        return "Any time";
      case "device_is":
        return `${device(c.device, "A device")} is ${c.on ? "on" : "off"}`;
      case "contact_is":
        return `${device(c.device, "A door or window")} is ${c.open ? "open" : "closed"}`;
    }
  }
  const a = step.value;
  switch (a.do) {
    case "set_device":
      return setting(device(a.device, "a device"), a);
    case "set_room":
      return setting(a.room ? `${humanize(a.room)} lights` : "a room's lights", a);
    case "notify": {
      const said = a.body ? `Say "${a.body}"` : "Say something";
      return a.room ? `${said} in ${humanize(a.room)}` : said;
    }
  }
}

/** What a device picker for this kind of step offers. */
type Use = "contact" | "switchable" | "button";

const USE: Partial<Record<StepKind, Use>> = {
  contact: "contact",
  contact_is: "contact",
  device_state: "switchable",
  device_is: "switchable",
  set_device: "switchable",
  device_action: "button",
};

/** A door or window. The class is not enough: a sensor Niles has not
    classified still says `open`, and the office door is one. */
export function isContact(device: Device): boolean {
  return device.class === "contact" || (device.state.open ?? null) !== null;
}

function fits(device: Device, use: Use): boolean {
  if (use === "contact") return isContact(device);
  const switchable = ["light", "outlet", "switch"].includes(device.class);
  if (use === "switchable") return switchable;
  // A remote or a dimmer has no class of its own: it is whatever is
  // left once lights, plugs and doors are taken out.
  return !switchable && !isContact(device);
}

export interface Choice {
  value: string;
  label: string;
}

/**
 * The devices a picker offers for a kind of step, by the name a rule
 * stores. `current` is kept even when nothing registered matches it —
 * a sensor whose battery died is still the one the rule is about.
 */
export function deviceChoices(devices: Device[], kind: StepKind, current?: string): Choice[] {
  const use = USE[kind];
  const names = new Set(use ? devices.filter((d) => fits(d, use)).map(ruleDeviceName) : []);
  if (current) names.add(current);
  return [...names]
    .map((value) => ({ value, label: deviceLabel(value) }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

/** Every room something is in, and `current` even when nothing is. */
export function roomChoices(devices: Device[], current?: string): Choice[] {
  const rooms = new Set(devices.map((d) => d.room).filter(Boolean));
  if (current) rooms.add(current);
  return [...rooms]
    .map((value) => ({ value, label: humanize(value) }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

function first(devices: Device[], kind: StepKind): string {
  return deviceChoices(devices, kind)[0]?.value ?? "";
}

export function defaultTrigger(kind: TriggerKind, devices: Device[]): Trigger {
  switch (kind) {
    case "contact":
      return { type: kind, device: first(devices, kind), open: true };
    case "device_state":
      return { type: kind, device: first(devices, kind), on: true };
    case "device_action":
      return { type: kind, device: first(devices, kind) };
    case "timer_fired":
      return { type: kind };
    case "presence":
      return { type: kind, state: "home" };
  }
}

export function defaultCondition(kind: ConditionKind, devices: Device[]): Condition {
  switch (kind) {
    case "time_of_day":
      return { kind, after: "07:00", before: "23:00" };
    case "device_is":
      return { kind, device: first(devices, kind), on: true };
    case "contact_is":
      return { kind, device: first(devices, kind), open: false };
  }
}

export function defaultAction(kind: ActionKind, devices: Device[]): Action {
  switch (kind) {
    case "set_room":
      return { do: kind, room: roomChoices(devices)[0]?.value ?? "", on: true };
    case "set_device":
      return { do: kind, device: first(devices, kind), on: true };
    case "notify":
      return { do: kind, body: "" };
  }
}

/**
 * A new rule, already the one most people come to make first: when
 * the door opens, turn off the lights in its room.
 */
export function newRule(devices: Device[]): Draft {
  const sensor = devices.find(isContact);
  return {
    description: "",
    enabled: true,
    trigger: { type: "contact", device: sensor ? ruleDeviceName(sensor) : "", open: true },
    conditions: [],
    actions: [{ do: "set_room", room: sensor?.room ?? "", on: false }],
  };
}

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;

function levels(where: string, a: { on?: boolean; brightness?: number; kelvin?: number }) {
  const found: string[] = [];
  if (a.on === undefined && a.brightness === undefined && a.kelvin === undefined) {
    found.push(`${where} does not say what to do.`);
  }
  if (
    a.brightness !== undefined &&
    (!Number.isInteger(a.brightness) || a.brightness < 0 || a.brightness > 100)
  ) {
    found.push(`${where}: brightness is 0 to 100%.`);
  }
  if (
    a.kelvin !== undefined &&
    (!Number.isInteger(a.kelvin) || a.kelvin < 2000 || a.kelvin > 6500)
  ) {
    found.push(`${where}: colour temperature is 2000 to 6500 K.`);
  }
  return found;
}

/**
 * Why this rule cannot be saved yet, in the order the line reads.
 *
 * The server checks the same and more; this is so Save can say what is
 * missing before anybody presses it, rather than after.
 */
export function problems(rule: Draft): string[] {
  const found: string[] = [];
  const t = rule.trigger;
  if ((t.type === "contact" || t.type === "device_action") && !t.device) {
    found.push("The trigger needs a device.");
  }
  rule.conditions.forEach((c, i) => {
    const where = `Condition ${i + 1}`;
    if (c.kind === "time_of_day") {
      if (!c.after && !c.before) found.push(`${where} needs a start or an end.`);
      for (const time of [c.after, c.before]) {
        if (time && !TIME.test(time)) found.push(`${where}: ${time} is not a time.`);
      }
    } else if (!c.device) {
      found.push(`${where} needs a device.`);
    }
  });
  if (rule.actions.length === 0) found.push("An automation needs at least one action.");
  rule.actions.forEach((a, i) => {
    const where = `Action ${i + 1}`;
    if (a.do === "set_device") {
      if (!a.device) found.push(`${where} needs a device.`);
      found.push(...levels(where, a));
    } else if (a.do === "set_room") {
      if (!a.room) found.push(`${where} needs a room.`);
      found.push(...levels(where, a));
    } else if (!a.body.trim()) {
      found.push(`${where} needs something to say.`);
    }
  });
  return found;
}

export interface StepData extends Record<string, unknown> {
  step: Step;
  summary: string;
}

export type StepNode = Node<StepData, "step">;

/** The size dagre lays a node out at; the node draws itself this wide. */
export const NODE_WIDTH = 256;
export const NODE_HEIGHT = 68;

/**
 * The rule as React Flow nodes and edges, laid out top to bottom.
 *
 * dagre for a straight line is more than it needs today, but it is what
 * keeps the picture right if a rule ever stops being one.
 */
export function toFlow(rule: Pick<Rule, "trigger" | "conditions" | "actions">): {
  nodes: StepNode[];
  edges: Edge[];
} {
  const steps = stepsOf(rule);
  const graph = new dagre.graphlib.Graph();
  graph.setGraph({ rankdir: "TB", nodesep: 40, ranksep: 48 });
  graph.setDefaultEdgeLabel(() => ({}));
  for (const step of steps) graph.setNode(step.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  const edges: Edge[] = steps.slice(1).map((step, i) => {
    const source = steps[i].id;
    graph.setEdge(source, step.id);
    return {
      id: `${source}->${step.id}`,
      source,
      target: step.id,
      markerEnd: { type: MarkerType.ArrowClosed, color: "var(--muted-foreground)" },
      style: { stroke: "var(--muted-foreground)", strokeWidth: 1.5 },
    };
  });
  dagre.layout(graph);
  const nodes = steps.map((step): StepNode => {
    const { x, y } = graph.node(step.id);
    return {
      id: step.id,
      type: "step",
      // dagre gives centres; React Flow wants top-left corners.
      position: { x: x - NODE_WIDTH / 2, y: y - NODE_HEIGHT / 2 },
      // Fixed, so the first fit has sizes to fit before anything is
      // measured.
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
      data: { step, summary: summary(step) },
    };
  });
  return { nodes, edges };
}
