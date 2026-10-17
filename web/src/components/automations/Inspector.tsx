import { ArrowDown, ArrowUp, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Adder } from "@/components/automations/Palette";
import { KIND_ICON } from "@/components/automations/StepNode";
import {
  ACTION_KINDS,
  BLINDS_POSITION,
  CONDITION_KINDS,
  DAYS,
  KIND_LABEL,
  measureChoices,
  measureHint,
  TRIGGER_KINDS,
  WAKE_UP_MINUTES,
  WAKE_UP_WARMTH,
  addAction,
  blindsPosition,
  blindsTarget,
  defaultAction,
  defaultCondition,
  defaultTrigger,
  deviceChoices,
  headingOf,
  kindOf,
  labelOf,
  moveStep,
  roomChoices,
  sensorChoices,
  siblings,
  stepAt,
  toggleDay,
  triggerChoices,
  wakeChoices,
  wakeTarget,
  watchesSeveral,
  withBlindsTarget,
  withSeveralSensors,
  withWakeTarget,
  withStep,
  withoutStep,
} from "@/lib/automations";
import type {
  ActionKind,
  BlindsPosition,
  BlindsTarget,
  Branch,
  Draft,
  Step,
  StepKind,
  WakeTarget,
  Watching,
} from "@/lib/automations";
import type { Action, Condition, Day, Device, Duration, Measure, Trigger } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface InspectorProps {
  rule: Draft;
  selected: string | null;
  devices: Device[];
  /** The rule as changed, and the step to have selected afterwards. */
  onChange: (rule: Draft, selected: string | null) => void;
}

/** What the trigger picker offers, in the order it offers them. */
export const TRIGGER_CHOICES = TRIGGER_KINDS.map((kind) => ({
  value: kind,
  label: KIND_LABEL.trigger[kind],
}));

/** What "Add to then" and "Add to else" offer on an `if` this deep
    (1 for one at the top): no further `if` from the third down, which
    is as deep as the server takes them. */
export function branchKinds(depth: number): ActionKind[] {
  return depth >= 3 ? ACTION_KINDS.filter((k) => k !== "if") : ACTION_KINDS;
}

/**
 * The side panel: the selected step's fields, and moving or removing
 * it. The last trigger can change kind but never goes; everything else
 * moves along its own list.
 */
export function Inspector({ rule, selected, devices, onChange }: InspectorProps) {
  const step = stepAt(rule, selected);
  if (!step) {
    return (
      <p className="text-muted-foreground text-sm">
        Pick a step in the flow to change it, or add a trigger, a condition or an action.
      </p>
    );
  }
  const kind = kindOf(step);
  const set = (next: Step) => onChange(withStep(rule, next), step.id);
  const count = siblings(rule, step);
  const lone = step.role === "trigger" && rule.triggers.length <= 1;
  const addTo = (branch: Branch) => (next: ActionKind) => {
    const added = addAction(rule, defaultAction(next, devices, rule.triggers), {
      id: step.id,
      branch,
    });
    onChange(added.rule, added.id);
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <span className="bg-muted text-muted-foreground flex size-8 shrink-0 items-center justify-center rounded-lg [&>svg]:size-4">
          {KIND_ICON[kind]}
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
            {headingOf(step)}
          </div>
          <div className="truncate text-sm font-medium">{labelOf(step)}</div>
        </div>
        {!lone && (
          <>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Move up"
              disabled={step.index === 0}
              onClick={() => {
                const moved = moveStep(rule, step.id, -1);
                onChange(moved.rule, moved.id);
              }}
            >
              <ArrowUp />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Move down"
              disabled={step.index === count - 1}
              onClick={() => {
                const moved = moveStep(rule, step.id, 1);
                onChange(moved.rule, moved.id);
              }}
            >
              <ArrowDown />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Remove step"
              onClick={() => onChange(withoutStep(rule, step.id), null)}
            >
              <Trash2 />
            </Button>
          </>
        )}
      </div>

      {step.role === "trigger" && (
        <Field label="Starts when">
          <Select
            items={TRIGGER_CHOICES}
            value={step.value.type}
            onValueChange={(next: Trigger["type"] | null) => {
              if (next && next !== step.value.type) {
                set({ ...step, value: defaultTrigger(next, devices) });
              }
            }}
          >
            <SelectTrigger aria-label="Trigger" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TRIGGER_CHOICES.map((choice) => (
                <SelectItem key={choice.value} value={choice.value}>
                  {choice.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
      )}
      {step.role === "trigger" && (
        <TriggerFields
          value={step.value}
          devices={devices}
          onChange={(value) => set({ ...step, value })}
        />
      )}
      {step.role === "condition" && (
        <ConditionFields
          value={step.value}
          devices={devices}
          triggers={rule.triggers}
          onChange={(value) => set({ ...step, value })}
        />
      )}
      {step.role === "action" && (
        <ActionFields
          value={step.value}
          devices={devices}
          triggers={rule.triggers}
          onChange={(value) => set({ ...step, value })}
        />
      )}
      {step.role === "action" && step.value.do === "if" && (
        <div className="flex flex-wrap gap-2">
          <Adder
            label="Add to then"
            kinds={branchKinds(step.id.split(".").length)}
            labels={KIND_LABEL.action}
            onPick={addTo("then")}
          />
          <Adder
            label="Add to else"
            kinds={branchKinds(step.id.split(".").length)}
            labels={KIND_LABEL.action}
            onPick={addTo("else")}
          />
        </div>
      )}
    </div>
  );
}

function TriggerFields({
  value,
  devices,
  onChange,
}: {
  value: Trigger;
  devices: Device[];
  onChange: (value: Trigger) => void;
}) {
  switch (value.type) {
    case "contact":
      return (
        <>
          <DeviceField
            label="Door or window"
            kind="contact"
            value={value.device}
            devices={devices}
            onChange={(device) => onChange({ ...value, device: device ?? "" })}
          />
          <Choices
            label="When it"
            value={value.open === undefined ? "either" : value.open ? "opens" : "closes"}
            options={[
              { value: "opens", label: "Opens" },
              { value: "closes", label: "Closes" },
              { value: "either", label: "Either" },
            ]}
            onChange={(next) =>
              onChange({
                ...value,
                open: next === "either" ? undefined : next === "opens",
              })
            }
          />
          <ForField value={value.for} onChange={(held) => onChange({ ...value, for: held })} />
        </>
      );
    case "motion": {
      const several = watchesSeveral(value);
      return (
        <>
          <SensorsField value={value} devices={devices} onChange={onChange} />
          {several ? (
            <Choices
              label="When"
              value={value.detected ? "any" : "none"}
              options={[
                { value: "any", label: "Any sees motion" },
                { value: "none", label: "None sees motion" },
              ]}
              onChange={(next) => onChange({ ...value, detected: next === "any" })}
            />
          ) : (
            <Choices
              label="When it"
              value={value.detected === undefined ? "either" : value.detected ? "sees" : "stops"}
              options={[
                { value: "sees", label: "Sees motion" },
                { value: "stops", label: "Stops seeing motion" },
                { value: "either", label: "Either" },
              ]}
              onChange={(next) =>
                onChange({
                  ...value,
                  detected: next === "either" ? undefined : next === "sees",
                })
              }
            />
          )}
          <ForField value={value.for} onChange={(held) => onChange({ ...value, for: held })} />
          {several && value.detected === false && (
            <p className="text-muted-foreground text-xs">
              The wait starts when the last of them clears; motion in any starts it over.
            </p>
          )}
        </>
      );
    }
    case "device_state":
      return (
        <>
          <DeviceField
            label="Device"
            kind="device_state"
            value={value.device}
            devices={devices}
            any="Any device"
            onChange={(device) => onChange({ ...value, device: device ?? undefined })}
          />
          {!value.device && (
            <RoomField
              label="In"
              value={value.room}
              devices={devices}
              any="Any room"
              onChange={(room) => onChange({ ...value, room: room ?? undefined })}
            />
          )}
          <Choices
            label="When it"
            value={value.on === undefined ? "either" : value.on ? "on" : "off"}
            options={[
              { value: "on", label: "Turns on" },
              { value: "off", label: "Turns off" },
              { value: "either", label: "Either" },
            ]}
            onChange={(next) =>
              onChange({
                ...value,
                on: next === "either" ? undefined : next === "on",
              })
            }
          />
          <ForField value={value.for} onChange={(held) => onChange({ ...value, for: held })} />
        </>
      );
    case "numeric":
      return (
        <>
          <ReadingFields
            value={value}
            devices={devices}
            onChange={(r) => onChange({ ...value, ...r })}
          />
          <ForField value={value.for} onChange={(held) => onChange({ ...value, for: held })} />
        </>
      );
    case "device_action":
      return (
        <>
          <DeviceField
            label="Button or device"
            kind="device_action"
            value={value.device}
            devices={devices}
            onChange={(device) => onChange({ ...value, device: device ?? "" })}
          />
          <Field label="Action">
            <Input
              aria-label="Action"
              placeholder="Any action, or e.g. on_press or finished"
              value={value.action ?? ""}
              onChange={(e) => onChange({ ...value, action: e.target.value.trim() || undefined })}
            />
          </Field>
        </>
      );
    case "timer_fired":
      return (
        <Field label="Timer">
          <Input
            aria-label="Timer"
            placeholder="Any timer"
            value={value.name ?? ""}
            onChange={(e) => onChange({ ...value, name: e.target.value || undefined })}
          />
        </Field>
      );
    case "presence":
      return (
        <>
          <Choices
            label="When"
            value={value.state ?? "either"}
            options={[
              { value: "home", label: "Somebody comes home" },
              { value: "away", label: "Everybody leaves" },
              { value: "either", label: "Either" },
            ]}
            onChange={(next) =>
              onChange({
                ...value,
                state: next === "either" ? undefined : (next as "home" | "away"),
              })
            }
          />
          <ForField value={value.for} onChange={(held) => onChange({ ...value, for: held })} />
        </>
      );
    case "time":
      return (
        <>
          <Field label="At">
            <Input
              type="time"
              aria-label="At"
              value={value.at}
              onChange={(e) => onChange({ ...value, at: e.target.value })}
            />
          </Field>
          <DaysField value={value.days} onChange={(days) => onChange({ ...value, days })} />
        </>
      );
    case "sun":
      return (
        <>
          <Choices
            label="At"
            value={value.event}
            options={[
              { value: "sunrise", label: "Sunrise" },
              { value: "sunset", label: "Sunset" },
            ]}
            onChange={(next) => onChange({ ...value, event: next as "sunrise" | "sunset" })}
          />
          <OffsetField
            label="Minutes after"
            value={value.offset_minutes}
            onChange={(offset) => onChange({ ...value, offset_minutes: offset })}
          />
        </>
      );
  }
}

function ConditionFields({
  value,
  devices,
  triggers,
  onChange,
}: {
  value: Condition;
  devices: Device[];
  triggers: Trigger[];
  onChange: (value: Condition) => void;
}) {
  switch (value.kind) {
    case "time_of_day":
      return (
        <>
          <div className="grid grid-cols-2 gap-3">
            <Field label="After">
              <Input
                type="time"
                aria-label="After"
                value={value.after ?? ""}
                onChange={(e) => onChange({ ...value, after: e.target.value || undefined })}
              />
            </Field>
            <Field label="Before">
              <Input
                type="time"
                aria-label="Before"
                value={value.before ?? ""}
                onChange={(e) => onChange({ ...value, before: e.target.value || undefined })}
              />
            </Field>
            <p className="text-muted-foreground col-span-2 text-xs">
              A window that ends before it starts runs past midnight.
            </p>
          </div>
          <DaysField value={value.days} onChange={(days) => onChange({ ...value, days })} />
        </>
      );
    case "sun":
      return (
        <>
          <SunEdge
            label="After"
            value={value.after}
            offset={value.after_offset_minutes}
            onChange={(after, offset) =>
              onChange({ ...value, after, after_offset_minutes: offset })
            }
          />
          <SunEdge
            label="Before"
            value={value.before}
            offset={value.before_offset_minutes}
            onChange={(before, offset) =>
              onChange({ ...value, before, before_offset_minutes: offset })
            }
          />
        </>
      );
    case "device_is":
      return (
        <>
          <DeviceField
            label="Device"
            kind="device_is"
            value={value.device}
            devices={devices}
            onChange={(device) => onChange({ ...value, device: device ?? "" })}
          />
          <Choices
            label="Is"
            value={value.on ? "on" : "off"}
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            onChange={(next) => onChange({ ...value, on: next === "on" })}
          />
        </>
      );
    case "contact_is":
      return (
        <>
          <DeviceField
            label="Door or window"
            kind="contact_is"
            value={value.device}
            devices={devices}
            onChange={(device) => onChange({ ...value, device: device ?? "" })}
          />
          <Choices
            label="Is"
            value={value.open ? "open" : "closed"}
            options={[
              { value: "open", label: "Open" },
              { value: "closed", label: "Closed" },
            ]}
            onChange={(next) => onChange({ ...value, open: next === "open" })}
          />
        </>
      );
    case "motion_is": {
      const several = watchesSeveral(value);
      return (
        <>
          <SensorsField value={value} devices={devices} onChange={onChange} />
          <Choices
            label="Is"
            value={value.detected ? "motion" : "none"}
            options={[
              { value: "motion", label: several ? "Any sees motion" : "Sees motion" },
              { value: "none", label: several ? "None sees motion" : "Sees no motion" },
            ]}
            onChange={(next) => onChange({ ...value, detected: next === "motion" })}
          />
        </>
      );
    }
    case "numeric":
      return (
        <ReadingFields
          value={value}
          devices={devices}
          onChange={(r) => onChange({ ...value, ...r })}
        />
      );
    case "presence_is":
      return (
        <Choices
          label="When"
          value={value.state}
          options={[
            { value: "home", label: "Somebody is home" },
            { value: "away", label: "Everybody is away" },
          ]}
          onChange={(next) => onChange({ ...value, state: next as "home" | "away" })}
        />
      );
    case "triggered_by":
      return (
        <StartedByField
          value={value.triggers}
          triggers={triggers}
          onChange={(ids) => onChange({ ...value, triggers: ids })}
        />
      );
  }
}

function ActionFields({
  value,
  devices,
  triggers,
  onChange,
}: {
  value: Action;
  devices: Device[];
  triggers: Trigger[];
  onChange: (value: Action) => void;
}) {
  switch (value.do) {
    case "set_device":
      return (
        <>
          <DeviceField
            label="Device"
            kind="set_device"
            value={value.device}
            devices={devices}
            onChange={(device) => onChange({ ...value, device: device ?? "" })}
          />
          <LevelFields value={value} onChange={(next) => onChange({ ...value, ...next })} />
        </>
      );
    case "set_room":
      return (
        <>
          <RoomField
            label="Every light in"
            kind="set_room"
            value={value.room}
            devices={devices}
            onChange={(room) => onChange({ ...value, room: room ?? "" })}
          />
          <LevelFields value={value} onChange={(next) => onChange({ ...value, ...next })} />
        </>
      );
    case "set_blinds": {
      const position = blindsPosition(value.position);
      return (
        <>
          <Choices
            label="Which"
            value={blindsTarget(value)}
            options={[
              { value: "room", label: "All in a room" },
              { value: "device", label: "One blind" },
            ]}
            onChange={(next) => onChange(withBlindsTarget(value, next as BlindsTarget, devices))}
          />
          {blindsTarget(value) === "room" ? (
            <RoomField
              label="Every blind in"
              kind="set_blinds"
              value={value.room}
              devices={devices}
              onChange={(room) => onChange({ ...value, room: room ?? "" })}
            />
          ) : (
            <DeviceField
              label="Blind"
              kind="set_blinds"
              value={value.device}
              devices={devices}
              onChange={(device) => onChange({ ...value, device: device ?? "" })}
            />
          )}
          <Choices
            label="To"
            value={position}
            options={[
              { value: "close", label: "Close" },
              { value: "open", label: "Open" },
              { value: "fully", label: "Fully open" },
              { value: "part", label: "Part open" },
            ]}
            onChange={(next) => {
              // Pressing "Part open" again must not throw away the
              // percentage already typed in.
              if (next !== position) {
                onChange({ ...value, position: BLINDS_POSITION[next as BlindsPosition] });
              }
            }}
          />
          {position === "part" && (
            <Field label="Open (%)">
              <Input
                type="number"
                inputMode="numeric"
                aria-label="Open"
                min={1}
                max={99}
                value={typeof value.position === "number" ? value.position : ""}
                onChange={(e) => {
                  const next = numberOrNothing(e.target.value);
                  if (next !== undefined) onChange({ ...value, position: next });
                }}
              />
            </Field>
          )}
        </>
      );
    }
    case "wake_up": {
      const target = wakeTarget(value);
      const choices = wakeChoices(devices, value);
      return (
        <>
          <Choices
            label="Wake"
            value={target}
            options={[
              { value: "house", label: "Whole house" },
              { value: "rooms", label: "Rooms" },
              { value: "lights", label: "Lights" },
            ]}
            onChange={(next) => onChange(withWakeTarget(value, next as WakeTarget, devices))}
          />
          {target === "rooms" && (
            <Ticks
              label="Every light in"
              choices={choices.rooms}
              value={value.rooms ?? []}
              onChange={(rooms) => onChange({ ...value, rooms })}
            />
          )}
          {(target === "lights" || (value.devices?.length ?? 0) > 0) && (
            <Ticks
              label={target === "lights" ? "These lights" : "And these lights"}
              choices={choices.lights}
              value={value.devices ?? []}
              onChange={(picked) => onChange({ ...value, devices: picked })}
            />
          )}
          {target !== "lights" && (
            <Ticks
              label="But not"
              choices={choices.except}
              value={value.except ?? []}
              onChange={(except) => onChange({ ...value, except })}
            />
          )}
          <Field label="Over (minutes)">
            <Input
              type="number"
              inputMode="numeric"
              aria-label="Minutes"
              min={1}
              max={120}
              placeholder={String(WAKE_UP_MINUTES)}
              value={value.minutes ?? ""}
              onChange={(e) => onChange({ ...value, minutes: numberOrNothing(e.target.value) })}
            />
          </Field>
          <Field label="Up to (%)">
            <Input
              type="number"
              inputMode="numeric"
              aria-label="Brightness"
              min={1}
              max={100}
              placeholder="100"
              value={value.brightness ?? ""}
              onChange={(e) => onChange({ ...value, brightness: numberOrNothing(e.target.value) })}
            />
          </Field>
          <Choices
            label="Warmth"
            value={value.kelvin === undefined ? "curve" : "own"}
            options={[
              { value: "curve", label: "Follow the curve" },
              { value: "own", label: "Its own" },
            ]}
            onChange={(next) => {
              if (next === "own" && value.kelvin === undefined) {
                onChange({ ...value, ...WAKE_UP_WARMTH });
              } else if (next === "curve") {
                onChange({ ...value, start_kelvin: undefined, kelvin: undefined });
              }
            }}
          />
          {value.kelvin !== undefined && (
            <div className="grid grid-cols-2 gap-3">
              <Field label="From (K)">
                <Input
                  type="number"
                  inputMode="numeric"
                  aria-label="Start warmth"
                  min={2000}
                  max={6500}
                  step={100}
                  placeholder={String(value.kelvin)}
                  value={value.start_kelvin ?? ""}
                  onChange={(e) =>
                    onChange({ ...value, start_kelvin: numberOrNothing(e.target.value) })
                  }
                />
              </Field>
              <Field label="To (K)">
                <Input
                  type="number"
                  inputMode="numeric"
                  aria-label="End warmth"
                  min={2000}
                  max={6500}
                  step={100}
                  value={value.kelvin}
                  onChange={(e) => {
                    const kelvin = numberOrNothing(e.target.value);
                    // Emptied while typing: keep it on its own warmth
                    // rather than flipping back to the curve under them.
                    if (kelvin !== undefined) onChange({ ...value, kelvin });
                  }}
                />
              </Field>
            </div>
          )}
          <p className="text-muted-foreground text-xs">
            Lights already on are left as they are. At the end they stay up until somebody changes
            them. Following the curve with the curve switched off leaves the colour as it is.
          </p>
        </>
      );
    }
    case "notify":
      return (
        <>
          <Field label="Say">
            <Input
              aria-label="Say"
              placeholder="The garden door is open"
              value={value.body}
              onChange={(e) => onChange({ ...value, body: e.target.value })}
            />
          </Field>
          <RoomField
            label="In"
            value={value.room}
            devices={devices}
            any="Wherever Niles would"
            onChange={(room) => onChange({ ...value, room: room ?? undefined })}
          />
          <Choices
            label="Priority"
            value={value.priority ?? "routine"}
            options={[
              { value: "routine", label: "Routine" },
              { value: "important", label: "Important" },
              { value: "urgent", label: "Urgent" },
            ]}
            onChange={(next) =>
              onChange({
                ...value,
                priority: next as "routine" | "important" | "urgent",
              })
            }
          />
        </>
      );
    case "delay":
      return (
        <DurationFields
          label="Wait for"
          value={value}
          onChange={(held) => onChange({ do: "delay", ...held })}
        />
      );
    case "if":
      return (
        <Field label="If all of these hold">
          <div className="flex flex-col gap-2">
            {value.conditions.length === 0 && (
              <p className="text-muted-foreground text-xs">
                With nothing to check, it always takes "then".
              </p>
            )}
            {value.conditions.map((condition, i) => (
              <div key={i} className="flex flex-col gap-3 rounded-lg border p-3">
                <div className="flex items-center gap-2">
                  <span className="text-muted-foreground [&>svg]:size-4">
                    {KIND_ICON[condition.kind]}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {KIND_LABEL.condition[condition.kind]}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Remove condition ${i + 1}`}
                    onClick={() =>
                      onChange({
                        ...value,
                        conditions: value.conditions.filter((_, j) => j !== i),
                      })
                    }
                  >
                    <Trash2 />
                  </Button>
                </div>
                <ConditionFields
                  value={condition}
                  devices={devices}
                  triggers={triggers}
                  onChange={(next) =>
                    onChange({
                      ...value,
                      conditions: value.conditions.map((c, j) => (j === i ? next : c)),
                    })
                  }
                />
              </div>
            ))}
            <Adder
              label="Add condition"
              kinds={CONDITION_KINDS}
              labels={KIND_LABEL.condition}
              className="self-start"
              onPick={(kind) =>
                onChange({
                  ...value,
                  conditions: [...value.conditions, defaultCondition(kind, devices, triggers)],
                })
              }
            />
          </div>
        </Field>
      );
  }
}

interface Levels {
  on?: boolean;
  brightness?: number;
  kelvin?: number;
}

/** On or off, and with on, how bright and how warm. Off clears both:
    a level on a light being turned off means nothing. */
function LevelFields({ value, onChange }: { value: Levels; onChange: (next: Levels) => void }) {
  return (
    <>
      <Choices
        label="Turn"
        value={value.on === undefined ? "" : value.on ? "on" : "off"}
        options={[
          { value: "on", label: "On" },
          { value: "off", label: "Off" },
        ]}
        onChange={(next) =>
          onChange(
            next === "on" ? { on: true } : { on: false, brightness: undefined, kelvin: undefined },
          )
        }
      />
      {value.on !== false && (
        <div className="grid grid-cols-2 gap-3">
          <Field label="Brightness (%)">
            <Input
              type="number"
              inputMode="numeric"
              aria-label="Brightness"
              min={0}
              max={100}
              placeholder="As it is"
              value={value.brightness ?? ""}
              onChange={(e) => onChange({ brightness: numberOrNothing(e.target.value) })}
            />
          </Field>
          <Field label="Warmth (K)">
            <Input
              type="number"
              inputMode="numeric"
              aria-label="Colour temperature"
              min={2000}
              max={6500}
              step={100}
              placeholder="As it is"
              value={value.kelvin ?? ""}
              onChange={(e) => onChange({ kelvin: numberOrNothing(e.target.value) })}
            />
          </Field>
        </div>
      )}
    </>
  );
}

interface Reading {
  device: string;
  measure: Measure;
  above?: number;
  below?: number;
}

/** Which device, which of its readings, and the values it is held
    against. Either value may be left out, not both. */
function ReadingFields({
  value,
  devices,
  onChange,
}: {
  value: Reading;
  devices: Device[];
  onChange: (next: Partial<Reading>) => void;
}) {
  return (
    <>
      <PickField
        label="Reading"
        value={value.measure}
        choices={measureChoices(devices, value.measure)}
        empty="Pick a reading"
        onChange={(measure) => measure && onChange({ measure: measure as Measure })}
      />
      <DeviceField
        label="Device"
        kind="numeric"
        measure={value.measure}
        value={value.device}
        devices={devices}
        onChange={(device) => onChange({ device: device ?? "" })}
      />
      <div className="grid grid-cols-2 gap-3">
        <Field label="Above">
          <Input
            type="number"
            aria-label="Above"
            step="any"
            value={value.above ?? ""}
            onChange={(e) => onChange({ above: numberOrNothing(e.target.value) })}
          />
        </Field>
        <Field label="Below">
          <Input
            type="number"
            aria-label="Below"
            step="any"
            value={value.below ?? ""}
            onChange={(e) => onChange({ below: numberOrNothing(e.target.value) })}
          />
        </Field>
        {measureHint(value.measure) && (
          <p className="text-muted-foreground col-span-2 text-xs">{measureHint(value.measure)}</p>
        )}
      </div>
    </>
  );
}

/** Hours, minutes and seconds; all three empty is none. */
function DurationFields({
  label,
  value,
  hint,
  onChange,
}: {
  label: string;
  value: Duration | undefined;
  hint?: string;
  onChange: (value: Duration | undefined) => void;
}) {
  const parts = [
    ["hours", "Hours"],
    ["minutes", "Minutes"],
    ["seconds", "Seconds"],
  ] as const;
  const change = (key: keyof Duration, raw: string) => {
    const next: Duration = {};
    for (const [part] of parts) {
      const n = part === key ? numberOrNothing(raw) : value?.[part];
      if (n !== undefined) next[part] = n;
    }
    onChange(Object.keys(next).length > 0 ? next : undefined);
  };
  return (
    <Field label={label}>
      <div className="grid grid-cols-3 gap-2">
        {parts.map(([part, name]) => (
          <div key={part} className="flex flex-col gap-1">
            <Input
              type="number"
              inputMode="numeric"
              aria-label={name}
              min={0}
              placeholder="0"
              value={value?.[part] ?? ""}
              onChange={(e) => change(part, e.target.value)}
            />
            <span className="text-muted-foreground text-xs">{name.toLowerCase()}</span>
          </div>
        ))}
      </div>
      {hint && <p className="text-muted-foreground text-xs">{hint}</p>}
    </Field>
  );
}

/** How long a state must hold before the trigger fires. Optional: none
    fires the moment it changes. */
function ForField({
  value,
  onChange,
}: {
  value: Duration | undefined;
  onChange: (value: Duration | undefined) => void;
}) {
  return (
    <DurationFields
      label="And has stayed so for"
      value={value}
      hint="Leave empty to start at once. At most 24 hours."
      onChange={onChange}
    />
  );
}

function OffsetField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number | undefined;
  onChange: (value: number | undefined) => void;
}) {
  return (
    <Field label={label}>
      <Input
        type="number"
        inputMode="numeric"
        aria-label={label}
        step={1}
        placeholder="0"
        value={value ?? ""}
        onChange={(e) => onChange(numberOrNothing(e.target.value) || undefined)}
      />
      <p className="text-muted-foreground text-xs">Negative is before it.</p>
    </Field>
  );
}

/** One end of a sun window: sunrise, sunset or open, and how far from
    it. Open clears the offset with it. */
function SunEdge({
  label,
  value,
  offset,
  onChange,
}: {
  label: string;
  value: "sunrise" | "sunset" | undefined;
  offset: number | undefined;
  onChange: (value: "sunrise" | "sunset" | undefined, offset: number | undefined) => void;
}) {
  return (
    <>
      <Choices
        label={label}
        value={value ?? "open"}
        options={[
          { value: "sunrise", label: "Sunrise" },
          { value: "sunset", label: "Sunset" },
          { value: "open", label: "Any" },
        ]}
        onChange={(next) =>
          next === "open"
            ? onChange(undefined, undefined)
            : onChange(next as "sunrise" | "sunset", offset)
        }
      />
      {value && (
        <OffsetField
          label={`${label}: minutes after ${value}`}
          value={offset}
          onChange={(next) => onChange(value, next)}
        />
      )}
    </>
  );
}

/** Seven toggles, Monday first; none on is every day. */
function DaysField({
  value,
  onChange,
}: {
  value: Day[] | undefined;
  onChange: (value: Day[] | undefined) => void;
}) {
  const on = new Set(value ?? []);
  return (
    <Field label="On">
      <div role="group" aria-label="Days" className="flex gap-1">
        {DAYS.map((day) => (
          <button
            key={day.value}
            type="button"
            aria-pressed={on.has(day.value)}
            onClick={() => onChange(toggleDay(value, day.value))}
            className={cn(
              "h-7 flex-1 rounded-md border text-xs transition-colors",
              "focus-visible:ring-ring/50 focus-visible:ring-3 focus-visible:outline-none",
              on.has(day.value)
                ? "border-primary bg-primary text-primary-foreground"
                : "border-border text-muted-foreground hover:text-foreground",
            )}
          >
            {day.label}
          </button>
        ))}
      </div>
      <p className="text-muted-foreground text-xs">None picked is every day.</p>
    </Field>
  );
}

/** The rule's triggers as boxes to tick, by place and sentence. The
    ids stay in the triggers' order, whatever order they were ticked. */
function StartedByField({
  value,
  triggers,
  onChange,
}: {
  value: string[];
  triggers: Trigger[];
  onChange: (value: string[]) => void;
}) {
  const choices = triggerChoices(triggers);
  const ticked = new Set(value);
  return (
    <Field label="Only if started by">
      <div className="flex flex-col gap-1.5">
        {choices.map((choice) => (
          <label key={choice.value} className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              className="accent-primary size-4 shrink-0"
              checked={ticked.has(choice.value)}
              onChange={(e) => {
                if (e.target.checked) ticked.add(choice.value);
                else ticked.delete(choice.value);
                onChange(choices.map((c) => c.value).filter((id) => ticked.has(id)));
              }}
            />
            <span className="min-w-0 truncate" title={choice.label}>
              {choice.label}
            </span>
          </label>
        ))}
      </div>
    </Field>
  );
}

/** The sensor a motion step watches, or the several it watches as one:
    named, and every motion sensor in some rooms. */
function SensorsField<T extends Watching>({
  value,
  devices,
  onChange,
}: {
  value: T;
  devices: Device[];
  onChange: (value: T) => void;
}) {
  const several = watchesSeveral(value);
  const choices = sensorChoices(devices, value);
  return (
    <>
      <Choices
        label="Watch"
        value={several ? "several" : "one"}
        options={[
          { value: "one", label: "One sensor" },
          { value: "several", label: "Several" },
        ]}
        onChange={(next) => onChange(withSeveralSensors(value, next === "several", devices))}
      />
      {several ? (
        <>
          <Ticks
            label="These sensors"
            choices={choices.sensors}
            value={value.devices ?? []}
            onChange={(picked) => onChange({ ...value, devices: picked })}
          />
          <Ticks
            label="And every sensor in"
            choices={choices.rooms}
            value={value.rooms ?? []}
            onChange={(rooms) => onChange({ ...value, rooms })}
          />
        </>
      ) : (
        <DeviceField
          label="Motion sensor"
          kind="motion"
          value={value.device}
          devices={devices}
          onChange={(device) => onChange({ ...value, device: device ?? "" })}
        />
      )}
    </>
  );
}

/** Tick boxes for a step that takes several of something; the value
    keeps the order the boxes are in. */
function Ticks({
  label,
  choices,
  value,
  onChange,
}: {
  label: string;
  choices: { value: string; label: string }[];
  value: string[];
  onChange: (value: string[]) => void;
}) {
  const ticked = new Set(value);
  return (
    <Field label={label}>
      <div role="group" aria-label={label} className="flex flex-col gap-1.5">
        {choices.length === 0 && <p className="text-muted-foreground text-xs">Nothing to pick.</p>}
        {choices.map((choice) => (
          <label key={choice.value} className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              className="accent-primary size-4 shrink-0"
              checked={ticked.has(choice.value)}
              onChange={(e) => {
                if (e.target.checked) ticked.add(choice.value);
                else ticked.delete(choice.value);
                onChange(choices.map((c) => c.value).filter((v) => ticked.has(v)));
              }}
            />
            <span className="min-w-0 truncate" title={choice.label}>
              {choice.label}
            </span>
          </label>
        ))}
      </div>
    </Field>
  );
}

function numberOrNothing(raw: string): number | undefined {
  if (raw.trim() === "") return undefined;
  const value = Number(raw);
  return Number.isFinite(value) ? value : undefined;
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-muted-foreground text-xs">{label}</span>
      {children}
    </div>
  );
}

/** A row of buttons, one pressed: for the two or three answers a step
    takes, where a dropdown would hide them. */
function Choices({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <Field label={label}>
      <div role="group" aria-label={label} className="bg-muted flex gap-1 rounded-lg p-1">
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            aria-pressed={option.value === value}
            onClick={() => onChange(option.value)}
            className={cn(
              "flex-1 rounded-md px-2 py-1 text-sm transition-colors",
              "focus-visible:ring-ring/50 focus-visible:ring-3 focus-visible:outline-none",
              option.value === value
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {option.label}
          </button>
        ))}
      </div>
    </Field>
  );
}

/** `any` names the choice of none, for a field that may be left open. */
function DeviceField({
  label,
  kind,
  measure,
  value,
  devices,
  any,
  onChange,
}: {
  label: string;
  kind: StepKind;
  measure?: Measure;
  value: string | undefined;
  devices: Device[];
  any?: string;
  onChange: (device: string | null) => void;
}) {
  const choices = deviceChoices(devices, kind, value || undefined, measure);
  return (
    <PickField
      label={label}
      value={value || null}
      choices={any ? [{ value: null, label: any }, ...choices] : choices}
      empty="Pick a device"
      onChange={onChange}
    />
  );
}

/** `kind` narrows the rooms to the ones the step can act on. */
function RoomField({
  label,
  kind,
  value,
  devices,
  any,
  onChange,
}: {
  label: string;
  kind?: StepKind;
  value: string | undefined;
  devices: Device[];
  any?: string;
  onChange: (room: string | null) => void;
}) {
  const choices = roomChoices(devices, value || undefined, kind);
  return (
    <PickField
      label={label}
      value={value || null}
      choices={any ? [{ value: null, label: any }, ...choices] : choices}
      empty="Pick a room"
      onChange={onChange}
    />
  );
}

function PickField({
  label,
  value,
  choices,
  empty,
  onChange,
}: {
  label: string;
  value: string | null;
  choices: { value: string | null; label: string }[];
  empty: string;
  onChange: (value: string | null) => void;
}) {
  return (
    <Field label={label}>
      <Select items={choices} value={value} onValueChange={(next: string | null) => onChange(next)}>
        <SelectTrigger aria-label={label} className="w-full">
          <SelectValue placeholder={empty} />
        </SelectTrigger>
        <SelectContent>
          {choices.map((choice) => (
            <SelectItem key={choice.value ?? ""} value={choice.value}>
              {choice.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );
}
