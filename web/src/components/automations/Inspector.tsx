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
import { KIND_ICON } from "@/components/automations/StepNode";
import {
  KIND_LABEL,
  ROLE_LABEL,
  TRIGGER_KINDS,
  defaultTrigger,
  deviceChoices,
  kindOf,
  moveStep,
  roomChoices,
  stepAt,
  withStep,
  withoutStep,
} from "@/lib/automations";
import type { Draft, Step, StepKind } from "@/lib/automations";
import type { Action, Condition, Device, Trigger } from "@/lib/api";
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
  label: KIND_LABEL[kind],
}));

/**
 * The side panel: the selected step's fields, and moving or removing
 * it. The trigger can change kind but never goes; conditions and
 * actions move along their part of the line.
 */
export function Inspector({ rule, selected, devices, onChange }: InspectorProps) {
  const step = stepAt(rule, selected);
  if (!step) {
    return (
      <p className="text-muted-foreground text-sm">
        Pick a step in the flow to change it, or add a condition or an action.
      </p>
    );
  }
  const kind = kindOf(step);
  const set = (next: Step) => onChange(withStep(rule, next), step.id);
  const count = step.role === "condition" ? rule.conditions.length : rule.actions.length;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <span className="bg-muted text-muted-foreground flex size-8 shrink-0 items-center justify-center rounded-lg [&>svg]:size-4">
          {KIND_ICON[kind]}
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
            {ROLE_LABEL[step.role]}
          </div>
          <div className="truncate text-sm font-medium">{KIND_LABEL[kind]}</div>
        </div>
        {step.role !== "trigger" && (
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
          onChange={(value) => set({ ...step, value })}
        />
      )}
      {step.role === "action" && (
        <ActionFields
          value={step.value}
          devices={devices}
          onChange={(value) => set({ ...step, value })}
        />
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
        </>
      );
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
        </>
      );
    case "device_action":
      return (
        <>
          <DeviceField
            label="Button or dimmer"
            kind="device_action"
            value={value.device}
            devices={devices}
            onChange={(device) => onChange({ ...value, device: device ?? "" })}
          />
          <Field label="Press">
            <Input
              aria-label="Press"
              placeholder="Any press, or e.g. on_press"
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
      );
  }
}

function ConditionFields({
  value,
  devices,
  onChange,
}: {
  value: Condition;
  devices: Device[];
  onChange: (value: Condition) => void;
}) {
  switch (value.kind) {
    case "time_of_day":
      return (
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
  }
}

function ActionFields({
  value,
  devices,
  onChange,
}: {
  value: Action;
  devices: Device[];
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
            value={value.room}
            devices={devices}
            onChange={(room) => onChange({ ...value, room: room ?? "" })}
          />
          <LevelFields value={value} onChange={(next) => onChange({ ...value, ...next })} />
        </>
      );
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
  value,
  devices,
  any,
  onChange,
}: {
  label: string;
  kind: StepKind;
  value: string | undefined;
  devices: Device[];
  any?: string;
  onChange: (device: string | null) => void;
}) {
  const choices = deviceChoices(devices, kind, value || undefined);
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

function RoomField({
  label,
  value,
  devices,
  any,
  onChange,
}: {
  label: string;
  value: string | undefined;
  devices: Device[];
  any?: string;
  onChange: (room: string | null) => void;
}) {
  const choices = roomChoices(devices, value || undefined);
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
