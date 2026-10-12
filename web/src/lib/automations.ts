import dagre from "@dagrejs/dagre";
import { MarkerType } from "@xyflow/react";
import type { Edge, Node } from "@xyflow/react";
import type {
  Action,
  Condition,
  Day,
  Device,
  Duration,
  Measure,
  Rule,
  Run,
  Trigger,
} from "@/lib/api";
import { humanize } from "@/lib/rooms";

/**
 * Rules as the Automations page draws and edits them.
 *
 * A rule reads top to bottom: its triggers side by side, then its
 * conditions, then its actions, where an `if` splits into two branches
 * that meet again at the next action. The flow is a picture of that and
 * nothing more, so edges and positions are worked out from the rule
 * every time rather than stored anywhere.
 */

export type TriggerKind = Trigger["type"];
export type ConditionKind = Condition["kind"];
export type ActionKind = Action["do"];
export type StepKind = TriggerKind | ConditionKind | ActionKind;
export type Branch = "then" | "else";

/** The parts of a rule a flow is drawn from. */
export type Parts = Pick<Rule, "triggers" | "conditions" | "actions">;

/**
 * One node of the flow, and where in the rule it lives.
 *
 * Ids are paths: `trigger-1`, `condition-0`, `action-2`, and for an
 * action inside an `if`, the `if`'s id and the branch, as in
 * `action-2.then-0` or `action-2.else-1.then-0`. `index` is the place in
 * whichever list the step sits in.
 */
export type Step =
  | { role: "trigger"; id: string; index: number; value: Trigger }
  | { role: "condition"; id: string; index: number; value: Condition }
  | { role: "action"; id: string; index: number; branch: Branch | null; value: Action };

export const TRIGGER_KINDS: TriggerKind[] = [
  "contact",
  "device_state",
  "numeric",
  "device_action",
  "presence",
  "time",
  "sun",
  "timer_fired",
];
export const CONDITION_KINDS: ConditionKind[] = [
  "time_of_day",
  "sun",
  "device_is",
  "contact_is",
  "numeric",
  "presence_is",
  "triggered_by",
];
export const ACTION_KINDS: ActionKind[] = ["set_room", "set_device", "notify", "delay", "if"];

/** What each kind is called on a node, in the palette and in the
    trigger picker. By role, because a sun trigger and a sun condition
    are different questions. */
export const KIND_LABEL: {
  trigger: Record<TriggerKind, string>;
  condition: Record<ConditionKind, string>;
  action: Record<ActionKind, string>;
} = {
  trigger: {
    contact: "Door or window",
    device_state: "Device turns on or off",
    numeric: "A reading changes",
    device_action: "Button press",
    presence: "Home or away",
    time: "Time of day",
    sun: "Sunrise or sunset",
    timer_fired: "Timer goes off",
  },
  condition: {
    time_of_day: "Time of day",
    sun: "Before or after sunset",
    device_is: "Device is on or off",
    contact_is: "Door or window is",
    numeric: "A reading is",
    presence_is: "Home or away",
    triggered_by: "Which trigger started it",
  },
  action: {
    set_room: "Lights in a room",
    set_device: "One device",
    notify: "Say something",
    delay: "Wait",
    if: "If, then, else",
  },
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

export function labelOf(step: Step): string {
  switch (step.role) {
    case "trigger":
      return KIND_LABEL.trigger[step.value.type];
    case "condition":
      return KIND_LABEL.condition[step.value.kind];
    case "action":
      return KIND_LABEL.action[step.value.do];
  }
}

/** What a node says above its summary: triggers by their place, since
    that is how the rest of the rule names them. */
export function headingOf(step: Step): string {
  if (step.role === "trigger") return triggerName(step.index);
  if (step.role === "action" && step.branch === "else") return "Else";
  return ROLE_LABEL[step.role];
}

export function triggerName(index: number): string {
  return `Trigger ${index + 1}`;
}

function actionSteps(actions: Action[], prefix: string, branch: Branch | null): Step[] {
  return actions.flatMap((value, index): Step[] => {
    const id = `${prefix}-${index}`;
    const own: Step = { role: "action", id, index, branch, value };
    if (value.do !== "if") return [own];
    return [
      own,
      ...actionSteps(value.then, `${id}.then`, "then"),
      ...actionSteps(value.else ?? [], `${id}.else`, "else"),
    ];
  });
}

/** Every step, in the order the rule reads: an `if`, then its `then`
    branch, then its `else`. */
export function stepsOf(rule: Parts): Step[] {
  return [
    ...rule.triggers.map(
      (value, index): Step => ({
        role: "trigger",
        id: `trigger-${index}`,
        index,
        value,
      }),
    ),
    ...rule.conditions.map(
      (value, index): Step => ({
        role: "condition",
        id: `condition-${index}`,
        index,
        value,
      }),
    ),
    ...actionSteps(rule.actions, "action", null),
  ];
}

export function stepAt(rule: Parts, id: string | null | undefined): Step | undefined {
  return stepsOf(rule).find((step) => step.id === id);
}

/** A rule being made has no id yet; everything else is the same. */
export type Draft = Omit<Rule, "id"> & { id?: string };

interface Segment {
  list: "action" | Branch;
  index: number;
}

const SEGMENT = /^(action|then|else)-(\d+)$/;

/** `action-2.then-0` → its segments, or nothing for anything that is
    not an action's id. */
function address(id: string): Segment[] | undefined {
  const segments: Segment[] = [];
  for (const part of id.split(".")) {
    const found = SEGMENT.exec(part);
    if (!found) return undefined;
    segments.push({ list: found[1] as Segment["list"], index: Number(found[2]) });
  }
  const [head, ...rest] = segments;
  if (head.list !== "action" || rest.some((s) => s.list === "action")) return undefined;
  return segments;
}

/** The list an addressed action sits in, if there is one. */
function listAt(actions: Action[], segments: Segment[]): Action[] | undefined {
  let list = actions;
  for (let i = 0; i < segments.length - 1; i++) {
    const parent = list[segments[i].index];
    if (parent?.do !== "if") return undefined;
    list = segments[i + 1].list === "else" ? (parent.else ?? []) : parent.then;
  }
  return list;
}

function withBranch(action: Action & { do: "if" }, branch: Branch, list: Action[]): Action {
  return branch === "then" ? { ...action, then: list } : { ...action, else: list };
}

/** `actions` with `change` made to the list the addressed action sits
    in, and everything around it as it was. */
function inList(
  actions: Action[],
  segments: Segment[],
  change: (list: Action[]) => Action[],
): Action[] {
  if (segments.length === 1) return change(actions);
  const [head, next, ...rest] = segments;
  const branch = next.list as Branch;
  return actions.map((a, i) => {
    if (i !== head.index || a.do !== "if") return a;
    return withBranch(
      a,
      branch,
      inList(branch === "then" ? a.then : (a.else ?? []), [next, ...rest], change),
    );
  });
}

/** The rule with one step's value replaced. A trigger keeps its id
    when the new value has none, so a change of kind does not cut it
    off from the conditions naming it. */
export function withStep<R extends Draft>(rule: R, step: Step): R {
  switch (step.role) {
    case "trigger":
      return {
        ...rule,
        triggers: rule.triggers.map((t, i) =>
          i === step.index ? { ...step.value, id: step.value.id ?? t.id } : t,
        ),
      };
    case "condition":
      return {
        ...rule,
        conditions: rule.conditions.map((c, i) => (i === step.index ? step.value : c)),
      };
    case "action": {
      const segments = address(step.id);
      if (!segments) return rule;
      return {
        ...rule,
        actions: inList(rule.actions, segments, (list) =>
          list.map((a, i) => (i === step.index ? step.value : a)),
        ),
      };
    }
  }
}

function withoutTriggerIn(conditions: Condition[], id: string): Condition[] {
  return conditions.map((c) =>
    c.kind === "triggered_by" ? { ...c, triggers: c.triggers.filter((t) => t !== id) } : c,
  );
}

function actionsWithoutTrigger(actions: Action[], id: string): Action[] {
  return actions.map((a) => {
    if (a.do !== "if") return a;
    const next: Action = {
      ...a,
      conditions: withoutTriggerIn(a.conditions, id),
      then: actionsWithoutTrigger(a.then, id),
    };
    return a.else ? { ...next, else: actionsWithoutTrigger(a.else, id) } : next;
  });
}

/**
 * The rule without a step. The last trigger stays: a rule always has
 * one, and changes its kind rather than going without. A trigger that
 * goes takes its id out of every `triggered_by`, however deep — a
 * condition left naming nobody is then for `problems` to point at.
 */
export function withoutStep<R extends Draft>(rule: R, id: string): R {
  const step = stepAt(rule, id);
  if (step?.role === "trigger") {
    if (rule.triggers.length <= 1) return rule;
    const gone = step.value.id;
    const triggers = rule.triggers.filter((_, i) => i !== step.index);
    if (!gone) return { ...rule, triggers };
    return {
      ...rule,
      triggers,
      conditions: withoutTriggerIn(rule.conditions, gone),
      actions: actionsWithoutTrigger(rule.actions, gone),
    };
  }
  if (step?.role === "condition") {
    return { ...rule, conditions: rule.conditions.filter((_, i) => i !== step.index) };
  }
  const segments = step?.role === "action" ? address(id) : undefined;
  if (step?.role === "action" && segments) {
    return {
      ...rule,
      actions: inList(rule.actions, segments, (list) => list.filter((_, i) => i !== step.index)),
    };
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

/** How many steps share a step's list, so the panel knows when it is
    already last. */
export function siblings(rule: Parts, step: Step): number {
  if (step.role === "trigger") return rule.triggers.length;
  if (step.role === "condition") return rule.conditions.length;
  const segments = address(step.id);
  return (segments && listAt(rule.actions, segments)?.length) ?? 0;
}

/** Move a step along its own list — a trigger among the triggers, an
    action within its branch — and the id it has afterwards, so it can
    stay selected. */
export function moveStep<R extends Draft>(
  rule: R,
  id: string,
  by: number,
): { rule: R; id: string } {
  const step = stepAt(rule, id);
  if (step?.role === "trigger") {
    const triggers = move(rule.triggers, step.index, by);
    const at = triggers === rule.triggers ? step.index : step.index + by;
    return { rule: { ...rule, triggers }, id: `trigger-${at}` };
  }
  if (step?.role === "condition") {
    const conditions = move(rule.conditions, step.index, by);
    const at = conditions === rule.conditions ? step.index : step.index + by;
    return { rule: { ...rule, conditions }, id: `condition-${at}` };
  }
  const segments = step?.role === "action" ? address(id) : undefined;
  const list = segments && listAt(rule.actions, segments);
  if (step?.role === "action" && segments && list) {
    const moved = move(list, step.index, by);
    if (moved === list) return { rule, id };
    const at = step.index + by;
    const prefix = id.slice(0, id.lastIndexOf("-"));
    return {
      rule: { ...rule, actions: inList(rule.actions, segments, () => moved) },
      id: `${prefix}-${at}`,
    };
  }
  return { rule, id };
}

/** The first `t1`, `t2`… no trigger of the rule has. */
export function nextTriggerId(triggers: Trigger[]): string {
  const taken = new Set(triggers.map((t) => t.id));
  let n = 1;
  while (taken.has(`t${n}`)) n++;
  return `t${n}`;
}

/** Every trigger with an id, so a condition can name any of them. The
    server gives saved ones theirs; this is for anything that came
    without. */
export function withTriggerIds<R extends Draft>(rule: R): R {
  if (rule.triggers.every((t) => t.id)) return rule;
  const triggers: Trigger[] = [];
  for (const t of rule.triggers) {
    triggers.push(t.id ? t : { ...t, id: nextTriggerId([...rule.triggers, ...triggers]) });
  }
  return { ...rule, triggers };
}

/** A new trigger after the others, with an id of its own. */
export function addTrigger<R extends Draft>(rule: R, trigger: Trigger): { rule: R; id: string } {
  const added = { ...trigger, id: trigger.id ?? nextTriggerId(rule.triggers) };
  return {
    rule: { ...rule, triggers: [...rule.triggers, added] },
    id: `trigger-${rule.triggers.length}`,
  };
}

export function addCondition<R extends Draft>(
  rule: R,
  condition: Condition,
): { rule: R; id: string } {
  return {
    rule: { ...rule, conditions: [...rule.conditions, condition] },
    id: `condition-${rule.conditions.length}`,
  };
}

/** A new action at the end of the rule, or, with `into`, at the end of
    one branch of the `if` with that id. */
export function addAction<R extends Draft>(
  rule: R,
  action: Action,
  into?: { id: string; branch: Branch },
): { rule: R; id: string } {
  if (!into) {
    return {
      rule: { ...rule, actions: [...rule.actions, action] },
      id: `action-${rule.actions.length}`,
    };
  }
  const step = stepAt(rule, into.id);
  const segments = address(into.id);
  if (step?.role !== "action" || step.value.do !== "if" || !segments) return { rule, id: into.id };
  const parent = step.value;
  const list = into.branch === "then" ? parent.then : (parent.else ?? []);
  return {
    rule: withStep(rule, {
      ...step,
      value: withBranch(parent, into.branch, [...list, action]),
    }),
    id: `${into.id}.${into.branch}-${list.length}`,
  };
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

function lowerFirst(text: string): string {
  return text.charAt(0).toLowerCase() + text.slice(1);
}

function upperFirst(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** "a", "a and b", "a, b and c". */
function listed(words: string[], joiner: "and" | "or"): string {
  if (words.length <= 1) return words.join("");
  return `${words.slice(0, -1).join(", ")} ${joiner} ${words[words.length - 1]}`;
}

export function totalSeconds(d: Duration): number {
  return (d.hours ?? 0) * 3600 + (d.minutes ?? 0) * 60 + (d.seconds ?? 0);
}

/** `{ hours: 1, minutes: 30 }` → "1 hour 30 minutes". */
export function durationText(d: Duration): string {
  const parts = (
    [
      [d.hours, "hour"],
      [d.minutes, "minute"],
      [d.seconds, "second"],
    ] as const
  )
    .filter(([n]) => n)
    .map(([n, unit]) => `${n} ${unit}${n === 1 ? "" : "s"}`);
  return parts.length > 0 ? parts.join(" ") : "no time";
}

function minutes(total: number): Duration {
  return { hours: Math.floor(total / 60), minutes: total % 60 };
}

/** "sunset", or "30 minutes before sunset". */
function sunPoint(event: "sunrise" | "sunset", offset: number | undefined): string {
  if (!offset) return event;
  return `${durationText(minutes(Math.abs(offset)))} ${offset < 0 ? "before" : "after"} ${event}`;
}

export const DAYS: { value: Day; label: string }[] = [
  { value: "mon", label: "Mon" },
  { value: "tue", label: "Tue" },
  { value: "wed", label: "Wed" },
  { value: "thu", label: "Thu" },
  { value: "fri", label: "Fri" },
  { value: "sat", label: "Sat" },
  { value: "sun", label: "Sun" },
];

const WEEKDAYS: Day[] = ["mon", "tue", "wed", "thu", "fri"];
const WEEKEND: Day[] = ["sat", "sun"];

/** "on weekdays", "on Mon and Thu", or nothing for every day. */
export function daysText(days: Day[] | undefined): string | null {
  const on = new Set(days ?? []);
  if (on.size === 0 || on.size === DAYS.length) return null;
  const exactly = (set: Day[]) => set.length === on.size && set.every((d) => on.has(d));
  if (exactly(WEEKDAYS)) return "on weekdays";
  if (exactly(WEEKEND)) return "at weekends";
  return `on ${listed(
    DAYS.filter((d) => on.has(d.value)).map((d) => d.label),
    "and",
  )}`;
}

/** The days in the week's order, whatever order they were picked in. */
export function toggleDay(days: Day[] | undefined, day: Day): Day[] | undefined {
  const on = new Set(days ?? []);
  if (on.has(day)) on.delete(day);
  else on.add(day);
  const next = DAYS.map((d) => d.value).filter((d) => on.has(d));
  return next.length > 0 ? next : undefined;
}

const UNIT: Record<Measure, string> = {
  temperature: "°",
  humidity: "%",
  battery: "%",
  brightness: "%",
};

export const MEASURE_LABEL: Record<Measure, string> = {
  temperature: "Temperature",
  humidity: "Humidity",
  battery: "Battery",
  brightness: "Brightness",
};

function bounds(measure: Measure, above?: number, below?: number): string {
  const unit = UNIT[measure];
  if (above !== undefined && below !== undefined) {
    return `between ${above}${unit} and ${below}${unit}`;
  }
  if (above !== undefined) return `above ${above}${unit}`;
  if (below !== undefined) return `below ${below}${unit}`;
  return "above or below a value";
}

function reading(name: string | undefined, measure: Measure): string {
  return `${device(name, "A sensor")} ${measure}`;
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

function triggerText(t: Trigger): string {
  switch (t.type) {
    case "contact": {
      const what = device(t.device, "A door or window");
      const state = t.open === undefined ? "open or closed" : t.open ? "open" : "closed";
      if (t.for) return `${what} has been ${state} for ${durationText(t.for)}`;
      if (t.open === undefined) return `${what} opens or closes`;
      return `${what} ${t.open ? "opens" : "closes"}`;
    }
    case "device_state": {
      const what = t.device
        ? deviceLabel(t.device)
        : t.room
          ? `Something in ${humanize(t.room)}`
          : "Any device";
      if (t.for) {
        const state = t.on === undefined ? "on or off" : t.on ? "on" : "off";
        return `${what} has been ${state} for ${durationText(t.for)}`;
      }
      return `${what} ${t.on === undefined ? "turns on or off" : t.on ? "turns on" : "turns off"}`;
    }
    case "numeric": {
      const what = reading(t.device, t.measure);
      const where = bounds(t.measure, t.above, t.below);
      return t.for ? `${what} has been ${where} for ${durationText(t.for)}` : `${what} ${where}`;
    }
    case "device_action": {
      const what = device(t.device, "A button");
      return t.action ? `${what}: ${t.action.replace(/_/g, " ")}` : `${what} is pressed`;
    }
    case "timer_fired":
      return t.name ? `The ${t.name} timer goes off` : "Any timer goes off";
    case "presence":
      if (t.for) {
        const held = durationText(t.for);
        return t.state === "home"
          ? `Somebody has been home for ${held}`
          : t.state === "away"
            ? `Everybody has been away for ${held}`
            : `Home or away has not changed for ${held}`;
      }
      return t.state === "home"
        ? "Somebody comes home"
        : t.state === "away"
          ? "Everybody leaves"
          : "Home or away changes";
    case "time": {
      const on = daysText(t.days);
      return `At ${t.at || "a time"}${on ? ` ${on}` : ""}`;
    }
    case "sun":
      return t.offset_minutes ? upperFirst(sunPoint(t.event, t.offset_minutes)) : `At ${t.event}`;
  }
}

/** Who a `triggered_by` means: "Trigger 1 or Trigger 3". */
function startedBy(ids: string[], triggers: Trigger[]): string {
  if (ids.length === 0) return "a trigger yet to be picked";
  return listed(
    ids.map((id) => {
      const at = triggers.findIndex((t) => t.id === id);
      return at === -1 ? "a trigger since removed" : triggerName(at);
    }),
    "or",
  );
}

/** A condition as a clause, capitalised: what has to be true. */
function conditionText(c: Condition, triggers: Trigger[]): string {
  switch (c.kind) {
    case "time_of_day": {
      const on = daysText(c.days);
      const window =
        c.after && c.before
          ? `Between ${c.after} and ${c.before}`
          : c.after
            ? `After ${c.after}`
            : c.before
              ? `Before ${c.before}`
              : null;
      if (window) return on ? `${window} ${on}` : window;
      return on ? upperFirst(on) : "Any time";
    }
    case "sun": {
      const after = c.after && sunPoint(c.after, c.after_offset_minutes);
      const before = c.before && sunPoint(c.before, c.before_offset_minutes);
      if (after && before) return `Between ${after} and ${before}`;
      if (after) return `After ${after}`;
      if (before) return `Before ${before}`;
      return "Any time of day";
    }
    case "device_is":
      return `${device(c.device, "A device")} is ${c.on ? "on" : "off"}`;
    case "contact_is":
      return `${device(c.device, "A door or window")} is ${c.open ? "open" : "closed"}`;
    case "numeric":
      return `${reading(c.device, c.measure)} is ${bounds(c.measure, c.above, c.below)}`;
    case "presence_is":
      return c.state === "home" ? "Somebody is home" : "Everybody is away";
    case "triggered_by":
      return `Started by ${startedBy(c.triggers, triggers)}`;
  }
}

function branchText(actions: Action[], triggers: Trigger[]): string {
  if (actions.length === 0) return "do nothing";
  return actions.map((a) => lowerFirst(actionText(a, triggers))).join(", ");
}

function actionText(a: Action, triggers: Trigger[]): string {
  switch (a.do) {
    case "set_device":
      return setting(device(a.device, "a device"), a);
    case "set_room":
      return setting(a.room ? `${humanize(a.room)} lights` : "a room's lights", a);
    case "notify": {
      const said = a.body ? `Say "${a.body}"` : "Say something";
      return a.room ? `${said} in ${humanize(a.room)}` : said;
    }
    case "delay":
      return totalSeconds(a) > 0 ? `Wait ${durationText(a)}` : "Wait";
    case "if": {
      const test =
        a.conditions.length > 0
          ? a.conditions.map((c) => lowerFirst(conditionText(c, triggers))).join(" and ")
          : "always";
      const otherwise = a.else && a.else.length > 0 ? ` else ${branchText(a.else, triggers)}` : "";
      return `If ${test} then ${branchText(a.then, triggers)}${otherwise}`;
    }
  }
}

/**
 * One line for a node: what this step does, in a few words.
 *
 * `triggers` are the rule's, so a condition that names a trigger can
 * say which by its place.
 */
export function summary(step: Step, triggers: Trigger[] = []): string {
  switch (step.role) {
    case "trigger":
      return triggerText(step.value);
    case "condition":
      return step.value.kind === "triggered_by"
        ? `Only if started by ${startedBy(step.value.triggers, triggers)}`
        : conditionText(step.value, triggers);
    case "action":
      return actionText(step.value, triggers);
  }
}

/** What the "started by" boxes offer: each trigger by its place and
    what it is, never its id. */
export function triggerChoices(triggers: Trigger[]): Choice[] {
  return triggers.flatMap((t, i) =>
    t.id ? [{ value: t.id, label: `${triggerName(i)} — ${triggerText(t)}` }] : [],
  );
}

/** What a device picker for this kind of step offers. */
type Use = "contact" | "switchable" | "button" | "reading";

const USE: Partial<Record<StepKind, Use>> = {
  contact: "contact",
  contact_is: "contact",
  device_state: "switchable",
  device_is: "switchable",
  set_device: "switchable",
  device_action: "button",
  numeric: "reading",
};

/** A door or window. The class is not enough: a sensor Niles has not
    classified still says `open`, and the office door is one. */
export function isContact(device: Device): boolean {
  return device.class === "contact" || (device.state.open ?? null) !== null;
}

/** Whether a device reports a measure. A light's brightness counts
    before it has said one: it is a light. */
function reads(device: Device, measure: Measure): boolean {
  const s = device.state;
  switch (measure) {
    case "temperature":
      return (s.temperature_celsius ?? null) !== null;
    case "humidity":
      return (s.humidity_percent ?? null) !== null;
    case "battery":
      return (s.battery_percent ?? null) !== null;
    case "brightness":
      return device.class === "light" || (s.brightness ?? null) !== null;
  }
}

const MEASURES: Measure[] = ["temperature", "humidity", "battery", "brightness"];

function fits(device: Device, use: Use, measure?: Measure): boolean {
  if (use === "contact") return isContact(device);
  if (use === "reading") {
    return measure ? reads(device, measure) : MEASURES.some((m) => reads(device, m));
  }
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

/** What the measure picker offers. */
export const MEASURE_CHOICES: Choice[] = MEASURES.map((m) => ({
  value: m,
  label: MEASURE_LABEL[m],
}));

/**
 * The devices a picker offers for a kind of step, by the name a rule
 * stores. `current` is kept even when nothing registered matches it —
 * a sensor whose battery died is still the one the rule is about. For a
 * reading, `measure` narrows it to the devices that report that one.
 */
export function deviceChoices(
  devices: Device[],
  kind: StepKind,
  current?: string,
  measure?: Measure,
): Choice[] {
  const use = USE[kind];
  const names = new Set(
    use ? devices.filter((d) => fits(d, use, measure)).map(ruleDeviceName) : [],
  );
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

function first(devices: Device[], kind: StepKind, measure?: Measure): string {
  return deviceChoices(devices, kind, undefined, measure)[0]?.value ?? "";
}

export function defaultTrigger(kind: TriggerKind, devices: Device[]): Trigger {
  switch (kind) {
    case "contact":
      return { type: kind, device: first(devices, kind), open: true };
    case "device_state":
      return { type: kind, device: first(devices, kind), on: true };
    case "numeric":
      return {
        type: kind,
        device: first(devices, kind, "temperature"),
        measure: "temperature",
        above: 24,
      };
    case "device_action":
      return { type: kind, device: first(devices, kind) };
    case "timer_fired":
      return { type: kind };
    case "presence":
      return { type: kind, state: "home" };
    case "time":
      return { type: kind, at: "07:00" };
    case "sun":
      return { type: kind, event: "sunset" };
  }
}

/** `triggers` are the rule's: "which trigger started it" starts out
    naming the first. */
export function defaultCondition(
  kind: ConditionKind,
  devices: Device[],
  triggers: Trigger[] = [],
): Condition {
  switch (kind) {
    case "time_of_day":
      return { kind, after: "07:00", before: "23:00" };
    case "sun":
      return { kind, after: "sunset", before: "sunrise" };
    case "device_is":
      return { kind, device: first(devices, kind), on: true };
    case "contact_is":
      return { kind, device: first(devices, kind), open: false };
    case "numeric":
      return {
        kind,
        device: first(devices, kind, "temperature"),
        measure: "temperature",
        above: 24,
      };
    case "presence_is":
      return { kind, state: "home" };
    case "triggered_by":
      return { kind, triggers: triggers[0]?.id ? [triggers[0].id] : [] };
  }
}

/** An `if` asks which trigger started the run when there is more than
    one — the reason most rules branch at all. */
export function defaultAction(
  kind: ActionKind,
  devices: Device[],
  triggers: Trigger[] = [],
): Action {
  switch (kind) {
    case "set_room":
      return { do: kind, room: roomChoices(devices)[0]?.value ?? "", on: true };
    case "set_device":
      return { do: kind, device: first(devices, kind), on: true };
    case "notify":
      return { do: kind, body: "" };
    case "delay":
      return { do: kind, minutes: 5 };
    case "if":
      return {
        do: kind,
        conditions: [
          defaultCondition(triggers.length > 1 ? "triggered_by" : "time_of_day", devices, triggers),
        ],
        then: [],
      };
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
    triggers: [
      { id: "t1", type: "contact", device: sensor ? ruleDeviceName(sensor) : "", open: true },
    ],
    conditions: [],
    actions: [{ do: "set_room", room: sensor?.room ?? "", on: false }],
  };
}

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;
const MOST_TRIGGERS = 10;
const LONGEST = 24 * 3600;
const DEEPEST = 3;

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

/** More than nothing, at most a day, and in whole numbers. */
function lasts(d: Duration): boolean {
  const parts = [d.hours, d.minutes, d.seconds].filter((p) => p !== undefined);
  const total = totalSeconds(d);
  return parts.every((p) => Number.isInteger(p) && p >= 0) && total > 0 && total <= LONGEST;
}

function conditionProblems(c: Condition, where: string, triggers: Trigger[]): string[] {
  const found: string[] = [];
  switch (c.kind) {
    case "time_of_day":
      if (!c.after && !c.before && !c.days?.length) {
        found.push(`${where} needs a start or an end.`);
      }
      for (const time of [c.after, c.before]) {
        if (time && !TIME.test(time)) found.push(`${where}: ${time} is not a time.`);
      }
      break;
    case "sun":
      if (!c.after && !c.before) found.push(`${where} needs a sunrise or a sunset.`);
      break;
    case "numeric":
      if (!c.device) found.push(`${where} needs a device.`);
      if (c.above === undefined && c.below === undefined) {
        found.push(`${where} needs a value to be above or below.`);
      }
      break;
    case "device_is":
    case "contact_is":
      if (!c.device) found.push(`${where} needs a device.`);
      break;
    case "presence_is":
      break;
    case "triggered_by": {
      const ids = new Set(triggers.map((t) => t.id).filter(Boolean));
      if (c.triggers.length === 0) found.push(`${where} needs a trigger to have started it.`);
      else if (c.triggers.some((id) => !ids.has(id))) {
        found.push(`${where} names a trigger that is gone.`);
      }
      break;
    }
  }
  return found;
}

function actionProblems(
  actions: Action[],
  place: (i: number) => string,
  depth: number,
  triggers: Trigger[],
): string[] {
  return actions.flatMap((a, i) => {
    const where = place(i);
    switch (a.do) {
      case "set_device":
        return [...(a.device ? [] : [`${where} needs a device.`]), ...levels(where, a)];
      case "set_room":
        return [...(a.room ? [] : [`${where} needs a room.`]), ...levels(where, a)];
      case "notify":
        return a.body.trim() ? [] : [`${where} needs something to say.`];
      case "delay":
        return lasts(a) ? [] : [`${where}: a wait is more than nothing and at most 24 hours.`];
      case "if":
        return [
          ...(depth > DEEPEST ? [`${where}: an if goes at most ${DEEPEST} deep.`] : []),
          ...a.conditions.flatMap((c, j) =>
            conditionProblems(c, `${where} › condition ${j + 1}`, triggers),
          ),
          ...actionProblems(a.then, (j) => `${where} › then ${j + 1}`, depth + 1, triggers),
          ...actionProblems(a.else ?? [], (j) => `${where} › else ${j + 1}`, depth + 1, triggers),
        ];
    }
  });
}

function triggerProblems(t: Trigger, where: string): string[] {
  const found: string[] = [];
  if ((t.type === "contact" || t.type === "device_action" || t.type === "numeric") && !t.device) {
    found.push(`${where} needs a device.`);
  }
  if (t.type === "numeric" && t.above === undefined && t.below === undefined) {
    found.push(`${where} needs a value to go above or below.`);
  }
  if (t.type === "time") {
    if (!t.at) found.push(`${where} needs a time.`);
    else if (!TIME.test(t.at)) found.push(`${where}: ${t.at} is not a time.`);
  }
  if ("for" in t && t.for && !lasts(t.for)) {
    found.push(`${where}: how long is more than nothing and at most 24 hours.`);
  }
  return found;
}

/**
 * Why this rule cannot be saved yet, in the order the flow reads.
 *
 * The server checks the same and more; this is so Save can say what is
 * missing before anybody presses it, rather than after.
 */
export function problems(rule: Draft): string[] {
  const found: string[] = [];
  if (rule.triggers.length === 0) found.push("An automation needs at least one trigger.");
  if (rule.triggers.length > MOST_TRIGGERS) {
    found.push(`An automation has at most ${MOST_TRIGGERS} triggers.`);
  }
  rule.triggers.forEach((t, i) => found.push(...triggerProblems(t, triggerName(i))));
  rule.conditions.forEach((c, i) =>
    found.push(...conditionProblems(c, `Condition ${i + 1}`, rule.triggers)),
  );
  if (rule.actions.length === 0) found.push("An automation needs at least one action.");
  found.push(...actionProblems(rule.actions, (i) => `Action ${i + 1}`, 1, rule.triggers));
  return found;
}

export interface StepData extends Record<string, unknown> {
  step: Step;
  heading: string;
  summary: string;
}

export type StepNode = Node<StepData, "step">;

/** The size dagre lays a node out at; the node draws itself this wide. */
export const NODE_WIDTH = 256;
export const NODE_HEIGHT = 68;

/** Where the next step's edges come from, and what the first of them
    says: an `if` hands on its branch's name. */
interface Exit {
  source: string;
  label?: Branch;
}

const EDGE_COLOUR = "var(--muted-foreground)";

/**
 * The rule as React Flow nodes and edges, laid out top to bottom.
 *
 * Every trigger joins into the first condition (or the first action);
 * an `if` joins into the first step of each branch, and the last steps
 * of both join into whatever comes after the `if`. An empty branch is
 * an edge straight past.
 */
export function toFlow(rule: Parts): { nodes: StepNode[]; edges: Edge[] } {
  const steps = stepsOf(rule);
  const graph = new dagre.graphlib.Graph();
  graph.setGraph({ rankdir: "TB", nodesep: 40, ranksep: 56 });
  graph.setDefaultEdgeLabel(() => ({}));
  for (const step of steps) graph.setNode(step.id, { width: NODE_WIDTH, height: NODE_HEIGHT });

  const edges: Edge[] = [];
  const join = (from: Exit[], target: string) => {
    for (const { source, label } of from) {
      graph.setEdge(source, target);
      edges.push({
        id: `${source}->${target}${label ? `:${label}` : ""}`,
        source,
        target,
        ...(label && {
          label,
          labelStyle: { fill: EDGE_COLOUR, fontSize: 11 },
          labelBgStyle: { fill: "var(--card)" },
          labelBgPadding: [4, 2] as [number, number],
        }),
        markerEnd: { type: MarkerType.ArrowClosed, color: EDGE_COLOUR },
        style: { stroke: EDGE_COLOUR, strokeWidth: 1.5 },
      });
    }
  };
  const chain = (actions: Action[], prefix: string, from: Exit[]): Exit[] => {
    let exits = from;
    actions.forEach((a, i) => {
      const id = `${prefix}-${i}`;
      join(exits, id);
      exits =
        a.do === "if"
          ? [
              ...chain(a.then, `${id}.then`, [{ source: id, label: "then" }]),
              ...chain(a.else ?? [], `${id}.else`, [{ source: id, label: "else" }]),
            ]
          : [{ source: id }];
    });
    return exits;
  };

  let exits: Exit[] = rule.triggers.map((_, i) => ({ source: `trigger-${i}` }));
  rule.conditions.forEach((_, i) => {
    join(exits, `condition-${i}`);
    exits = [{ source: `condition-${i}` }];
  });
  chain(rule.actions, "action", exits);

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
      data: { step, heading: headingOf(step), summary: summary(step, rule.triggers) },
    };
  });
  return { nodes, edges };
}

export const OUTCOME_LABEL: Record<Run["outcome"], string> = {
  running: "Running",
  done: "Done",
  stopped: "Stopped",
  replaced: "Replaced",
  failed: "Failed",
};

const WEEKDAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function midnight(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/**
 * When a run started, in local time and the way it is said: "Today
 * 14:02", "Yesterday 22:10", "Mon 08:30" within the week, "6 Oct
 * 08:30" before that. Spelled out rather than left to the locale, so it
 * reads the same as the rest of the page.
 */
export function ranAt(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  // Rounded, because a day that changes the clock is 23 or 25 hours.
  const days = Math.round((midnight(now) - midnight(at)) / 86_400_000);
  if (days === 0) return `Today ${time}`;
  if (days === 1) return `Yesterday ${time}`;
  if (days > 1 && days < 7) return `${WEEKDAY_NAMES[at.getDay()]} ${time}`;
  return `${at.getDate()} ${MONTH_NAMES[at.getMonth()]} ${time}`;
}

/** The trigger a run names, by its place in the rule; nothing when the
    run names none, or one the rule no longer has. */
export function runTrigger(run: Run, triggers: Trigger[]): string | null {
  if (!run.trigger) return null;
  const at = triggers.findIndex((t) => t.id === run.trigger);
  return at === -1 ? null : triggerName(at);
}

/** "Last run today 14:02 — done", "… — stopped: it was before 07:00". */
export function lastRunText(run: Run, now: Date = new Date()): string {
  const when = ranAt(run.started_at, now).replace(/^(Today|Yesterday)/, (w) => w.toLowerCase());
  const why = run.reason ? `: ${run.reason}` : "";
  return `Last run ${when} — ${run.outcome}${why}`;
}
