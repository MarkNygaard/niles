import { useMemo, useRef } from "react";
import { Combobox } from "@base-ui/react/combobox";
import { Check, ChevronDown, X } from "lucide-react";
import { cn } from "@/lib/utils";

export interface DeviceOption {
  /** What gets stored: a fully qualified id, `wled:living_room/tv`. */
  value: string;
  /** The device, as a person says it: "TV lightstrip". */
  label: string;
  /** Where it is, for telling two "lamp"s apart. */
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
 * The ids are the thing that actually goes in the config, and getting
 * one wrong fails silently — the light simply carries on following the
 * curve. So the list is the registry's own, and what you pick is what
 * is stored.
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

  // The popup anchors to whatever it is told to; left alone it picks
  // the inner text input, which is narrower than the box you see.
  const anchor = useRef<HTMLDivElement | null>(null);

  const selected = value.map((device) => byId.get(device)!);
  const labelFor = (device: string) => byId.get(device)?.label ?? device;

  return (
    <Combobox.Root
      items={options}
      multiple
      value={selected}
      disabled={disabled}
      itemToStringLabel={(item) => `${item.label} ${item.room} ${item.value}`}
      isItemEqualToValue={(item, other) => item.value === other.value}
      onValueChange={(next) => onChange(next.map((item) => item.value))}
    >
      <Combobox.Chips
        ref={anchor}
        className={cn(
          "border-input flex min-h-8 w-full flex-wrap items-center gap-1 rounded-lg border px-1.5 py-1 transition-colors",
          "focus-within:border-ring focus-within:ring-ring/50 focus-within:ring-3",
          "dark:bg-input/30 disabled:opacity-50",
        )}
      >
        {value.map((device) => (
          <Combobox.Chip
            key={device}
            aria-label={labelFor(device)}
            className={cn(
              "bg-secondary text-secondary-foreground flex items-center gap-1 rounded-md py-0.5 pr-0.5 pl-2 text-xs",
              // A device the registry doesn't know is worth seeing, not
              // worth hiding: it is probably a typo or a dead light.
              byId.get(device)?.source === "" && "text-muted-foreground border-input border border-dashed",
            )}
            title={device}
          >
            {labelFor(device)}
            <Combobox.ChipRemove
              className="hover:bg-muted rounded-sm p-0.5"
              aria-label={`Remove ${labelFor(device)}`}
            >
              <X className="size-3" />
            </Combobox.ChipRemove>
          </Combobox.Chip>
        ))}

        <Combobox.Input
          id={id}
          aria-label={ariaLabel}
          placeholder={value.length === 0 ? "Search lights…" : ""}
          className="text-foreground placeholder:text-muted-foreground min-w-32 flex-1 bg-transparent px-1 py-0.5 text-sm outline-none"
        />
        <Combobox.Trigger
          className="text-muted-foreground hover:text-foreground ml-auto rounded-sm p-1"
          aria-label="Show all lights"
        >
          <ChevronDown className="size-4" />
        </Combobox.Trigger>
      </Combobox.Chips>

      <Combobox.Portal>
        <Combobox.Positioner
          anchor={anchor}
          align="start"
          sideOffset={6}
          className="z-50"
        >
          <Combobox.Popup className="bg-popover text-popover-foreground border-border max-h-64 w-(--anchor-width) overflow-y-auto rounded-lg border p-1 shadow-lg">
            <Combobox.Empty className="text-muted-foreground px-2 py-3 text-xs">
              {options.length === 0 ? emptyMessage : "No light matches that."}
            </Combobox.Empty>
            <Combobox.List>
              {(option: DeviceOption) => (
                <Combobox.Item
                  key={option.value}
                  value={option.value}
                  className="data-highlighted:bg-muted flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-sm"
                >
                  <Combobox.ItemIndicator className="text-muted-foreground">
                    <Check className="size-3.5" />
                  </Combobox.ItemIndicator>
                  <span className="flex-1">{option.label}</span>
                  <span className="text-muted-foreground text-xs">
                    {option.room}
                    {option.source !== "z2m" && ` · ${option.source}`}
                  </span>
                </Combobox.Item>
              )}
            </Combobox.List>
          </Combobox.Popup>
        </Combobox.Positioner>
      </Combobox.Portal>
    </Combobox.Root>
  );
}

/**
 * A configured device the registry has never mentioned — a typo, or a
 * light that is currently offline. Shown as-is rather than dropped.
 */
function unregistered(device: string): DeviceOption {
  return { value: device, label: device, room: "not registered", source: "" };
}

/** Turn `GET /devices` into pickable options: lights, nicely named. */
export function deviceOptions(
  devices: Array<{ id: string; room: string; name: string; source: string; class: string }>,
): DeviceOption[] {
  return devices
    .filter((device) => device.class === "light")
    .map((device) => ({
      value: device.id,
      label: humanize(device.name),
      room: humanize(device.room),
      source: device.source,
    }))
    .sort((a, b) => a.room.localeCompare(b.room) || a.label.localeCompare(b.label));
}

/** `tv_lightstrip` → `Tv lightstrip`. Ids are snake_case by rule. */
function humanize(raw: string): string {
  const spaced = raw.replace(/_/g, " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}
