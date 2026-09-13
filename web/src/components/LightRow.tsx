import { useEffect, useState } from "react";
import {
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Slider,
  SliderControl,
  SliderIndicator,
  SliderThumb,
  SliderTrack,
} from "@/components/ui/slider";
import { ColorWheel, parseHex, toHex } from "@/components/ColorField";
import {
  ColorSwatch,
  KELVIN_MAX,
  KELVIN_MIN,
  KelvinSwatch,
} from "@/components/LightSwatches";
import { PowerButton } from "@/components/PowerButton";
import { brightnessOf, humanize, isDimmable } from "@/lib/rooms";
import type { Device, SetLight } from "@/lib/api";

export interface LightRowProps {
  light: Device;
  disabled?: boolean;
  onSet: (body: SetLight) => void;
}

/**
 * One light, with everything it can be told.
 *
 * Only what this light can actually do is offered: a strip with no
 * white channel has no colour temperature button, because a control
 * that publishes a value nothing acts on looks broken rather than
 * unsupported.
 */
export function LightRow({ light, disabled, onSet }: LightRowProps) {
  const on = light.state.on;
  const color = light.state.rgb ? toHex(light.state.rgb) : undefined;
  const dimmable = isDimmable(light);

  return (
    <div className="flex flex-col gap-2 py-3">
      <div className="flex items-center gap-3">
        <PowerButton
          on={on}
          disabled={disabled}
          label={humanize(light.name)}
          onToggle={(next) => onSet({ on: next })}
        />

        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">
            {humanize(light.name)}
          </div>
          <div className="text-muted-foreground text-xs">
            {on === null
              ? "Not heard from yet"
              : on && dimmable
                ? `On · ${brightnessOf(light)}%`
                : on
                  ? "On"
                  : "Off"}
            {!dimmable && " · plug"}
            {light.source !== "z2m" && ` · ${light.source}`}
          </div>
        </div>

        {light.supports_rgb && (
          <Control label="Colour" face={<ColorSwatch value={color} />} disabled={disabled}>
            <ColorWheel
              value={color ?? ""}
              disabled={disabled}
              onChange={(next) => {
                const rgb = parseHex(next);
                if (rgb) onSet({ rgb });
              }}
            />
          </Control>
        )}

        {light.supports_color_temp && (
          <Control
            label="Colour temperature"
            face={<KelvinSwatch value={light.state.color_temp_kelvin ?? undefined} />}
            disabled={disabled}
          >
            <CommitSlider
              label="Colour temperature"
              value={light.state.color_temp_kelvin ?? 2700}
              min={KELVIN_MIN}
              max={KELVIN_MAX}
              step={50}
              format={(v) => `${v}K`}
              // The track is the range it spans, so the thumb sits on
              // the white it is about to set.
              trackClassName="bg-[linear-gradient(90deg,rgb(255,160,0),rgb(255,255,255),rgb(166,209,255))]"
              indicatorClassName="bg-transparent"
              onCommit={(kelvin) => onSet({ color_temp_kelvin: kelvin })}
            />
          </Control>
        )}
      </div>

      {/* A plug has one thing it can be told; a slider that publishes
          a level nothing acts on would look broken, not unsupported. */}
      {dimmable && (
        <CommitSlider
          label={`${humanize(light.name)} brightness`}
          value={brightnessOf(light)}
          min={1}
          max={100}
          disabled={disabled}
          // Sending a brightness to a light that is off turns it on at
          // that level, which is what dragging this while off means.
          onCommit={(brightness) => onSet({ brightness })}
        />
      )}
    </div>
  );
}

function Control({
  label,
  face,
  disabled,
  children,
}: {
  label: string;
  face: React.ReactNode;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Popover>
      <PopoverTrigger
        disabled={disabled}
        aria-label={label}
        className="ring-border flex size-9 shrink-0 items-center justify-center rounded-full ring-1 transition-[box-shadow] hover:ring-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50"
      >
        {face}
      </PopoverTrigger>
      <PopoverContent className="w-64">
        <PopoverTitle className="mb-3">{label}</PopoverTitle>
        {children}
      </PopoverContent>
    </Popover>
  );
}

/**
 * A slider that sends when you let go.
 *
 * Every position under the thumb is a real MQTT message to a real
 * light. Committing continuously would flood the broker with values
 * nobody meant to set, and the light would visibly chase the thumb.
 */
function CommitSlider({
  label,
  value,
  min,
  max,
  step,
  disabled,
  format,
  trackClassName,
  indicatorClassName,
  onCommit,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  format?: (value: number) => string;
  trackClassName?: string;
  indicatorClassName?: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [dragging, setDragging] = useState(false);
  // While a finger is down, the light's own reports would yank the
  // thumb back to where it currently is. After that, follow the light:
  // the curve and voice move it too.
  useEffect(() => {
    if (!dragging) setDraft(value);
  }, [value, dragging]);

  return (
    <Slider
      value={draft}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      onValueChange={(next) => {
        setDragging(true);
        setDraft(next);
      }}
      onValueCommitted={(next) => {
        setDragging(false);
        onCommit(next);
      }}
    >
      <SliderControl>
        <SliderTrack className={trackClassName}>
          <SliderIndicator className={indicatorClassName} />
          {/* Base UI puts the range input inside the thumb, so the
              label belongs here rather than on the root. */}
          <SliderThumb
            aria-label={label}
            aria-valuetext={format ? format(draft) : `${draft}%`}
          />
        </SliderTrack>
      </SliderControl>
    </Slider>
  );
}
