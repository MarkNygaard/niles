import { useMemo, useRef } from "react";
import {
  Combobox,
  ComboboxChip,
  ComboboxChipRemove,
  ComboboxChips,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxItemIndicator,
  ComboboxList,
  ComboboxTrigger,
} from "@/components/ui/combobox";
import { Check, ChevronDown, X } from "lucide-react";
import { cn } from "@/lib/utils";

export interface DeviceOption {
  /** What gets stored: a fully qualified id, `wled:living_room/tv`. */
  value: string;
  /** The device, as a person says it: "Tv light". */
  label: string;
  /** Where it is. Empty for a device the registry doesn't know. */
  room: string;
  /** Which integration it comes from. */
  source: string;
}

export interface DevicePickerProps {
  id: string;
  "aria-label": string;
  value: string[];
  options: DeviceOption[];
  disabled?: boolean;
  /** Shown when the list is still loading, or nothing is registered. */
  emptyMessage: string;
  onChange: (value: string[]) => void;
}

/**
 * Pick lights by name instead of typing their ids.
 *
 * The ids are what actually goes in the config, and getting one wrong
 * fails silently — the light simply carries on following the curve. So
 * the list is the registry's own, and what you pick is what is stored.
 *
 * A device that is in the config but no longer registered still shows
 * as a chip. Dropping it on sight would quietly rewrite the config
 * whenever a light was offline.
 */
export function DevicePicker({
  id,
  "aria-label": ariaLabel,
  value,
  options,
  disabled,
  emptyMessage,
  onChange,
}: DevicePickerProps) {
  // Base UI holds whole options, not ids, so that it can label and
  // filter them. Both sides of that map come from one table, which also
  // makes the objects reference-equal — how items are matched to values.
  const byId = useMemo(() => {
    const table = new Map(options.map((option) => [option.value, option]));
    for (const device of value) {
      if (!table.has(device)) table.set(device, unregistered(device));
    }
    return table;
  }, [options, value.join("|")]);

  // The popup measures whatever it is anchored to; left alone that is
  // the inner input, which is narrower than the box you see.
  const anchor = useRef<HTMLDivElement | null>(null);

  const selected = value.map((device) => byId.get(device)!);
  const nameOf = (device: string) => {
    const option = byId.get(device);
    if (!option) return device;
    return option.room ? `${option.room} ${option.label}` : option.label;
  };

  return (
    <Combobox
      items={options}
      multiple
      value={selected}
      disabled={disabled}
      itemToStringLabel={(item) => `${item.label} ${item.room} ${item.value}`}
      isItemEqualToValue={(item, other) => item.value === other.value}
      onValueChange={(next) => onChange(next.map((item) => item.value))}
    >
      <ComboboxChips ref={anchor}>
        {value.map((device) => (
          <ComboboxChip
            key={device}
            aria-label={nameOf(device)}
            title={device}
            className={cn(
              // A device the registry doesn't know is worth seeing, not
              // worth hiding: it is probably a typo or a dead light.
              !byId.get(device)?.room &&
                "border-input text-muted-foreground border border-dashed",
            )}
          >
            {byId.get(device)?.room && (
              <span className="text-muted-foreground">
                {byId.get(device)!.room}
              </span>
            )}
            {byId.get(device)?.label ?? device}
            <ComboboxChipRemove aria-label={`Remove ${nameOf(device)}`}>
              <X className="size-3" />
            </ComboboxChipRemove>
          </ComboboxChip>
        ))}

        <ComboboxInput
          id={id}
          aria-label={ariaLabel}
          placeholder={value.length === 0 ? "Search lights…" : ""}
        />
        <ComboboxTrigger aria-label="Show all lights">
          <ChevronDown />
        </ComboboxTrigger>
      </ComboboxChips>

      <ComboboxContent anchor={anchor}>
        <ComboboxEmpty>
          {options.length === 0 ? emptyMessage : "No light matches that."}
        </ComboboxEmpty>
        <ComboboxList>
          {(option: DeviceOption) => (
            <ComboboxItem
              key={option.value}
              // The whole option, not its id: the root's value type is
              // the option, and a bare string silently never matches one.
              value={option}
            >
              <ComboboxItemIndicator>
                <Check />
              </ComboboxItemIndicator>
              <span className="flex-1">{option.label}</span>
              <span className="text-muted-foreground text-xs">
                {option.room}
                {option.source !== "z2m" && ` · ${option.source}`}
              </span>
            </ComboboxItem>
          )}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}

/**
 * A configured device the registry has never mentioned — a typo, or a
 * light that is currently offline. Shown as-is rather than dropped.
 */
function unregistered(device: string): DeviceOption {
  // No room: the id is shown whole, and the dashed chip says the rest.
  return { value: device, label: device, room: "", source: "" };
}

/** Turn `GET /devices` into pickable options: lights, nicely named. */
export function deviceOptions(
  devices: Array<{
    id: string;
    room: string;
    name: string;
    source: string;
    class: string;
  }>,
): DeviceOption[] {
  return devices
    .filter((device) => device.class === "light")
    .map((device) => ({
      value: device.id,
      label: humanize(device.name),
      room: humanize(device.room),
      source: device.source,
    }))
    .sort(
      (a, b) => a.room.localeCompare(b.room) || a.label.localeCompare(b.label),
    );
}

/** `tv_lightstrip` → `Tv lightstrip`. Ids are snake_case by rule. */
function humanize(raw: string): string {
  const spaced = raw.replace(/_/g, " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}
