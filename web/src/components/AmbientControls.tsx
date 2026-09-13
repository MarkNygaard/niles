import { useEffect, useState } from "react";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Slider,
  SliderControl,
  SliderIndicator,
  SliderThumb,
  SliderTrack,
  SliderValue,
} from "@/components/ui/slider";
import { ColorWheel } from "@/components/ColorField";
import {
  BrightnessGlyph,
  ColorSwatch,
  KELVIN_MAX,
  KELVIN_MIN,
  KelvinSwatch,
} from "@/components/LightSwatches";
import { cn } from "@/lib/utils";

export interface AmbientControlsProps {
  /** What the chosen ambient lights can be told to do. */
  supportsRgb: boolean;
  supportsColorTemp: boolean;
  brightness?: number;
  /** `#rrggbb`, or undefined when nothing is set. */
  color?: string;
  kelvin?: number;
  disabled?: boolean;
  onChange: (path: string, value: unknown) => void;
}

/**
 * The three things an ambient light is held at, each behind its own
 * button.
 *
 * Each button *is* its value — the brightness dial fills, the colour
 * disc shows the colour, the temperature disc shows the white. So the
 * row reads at a glance without opening anything, and opening one is
 * for changing it rather than seeing it.
 */
export function AmbientControls({
  supportsRgb,
  supportsColorTemp,
  brightness,
  color,
  kelvin,
  disabled,
  onChange,
}: AmbientControlsProps) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Control
        label="Brightness"
        hint="How bright ambient lights sit, as a percentage."
        disabled={disabled}
        summary={brightness === undefined ? "not set" : `${brightness}%`}
        face={<BrightnessGlyph />}
      >
        <ValueSlider
          label="Brightness"
          value={brightness}
          min={1}
          max={100}
          fallback={25}
          unit="%"
          onCommit={(next) => onChange("lighting.ambient_brightness", next)}
        />
      </Control>

      {supportsRgb && (
      <Control
        label="Colour"
        hint="Applied to the ambient lights that can take a colour."
        disabled={disabled}
        summary={color ?? "not set"}
        face={<ColorSwatch value={color} />}
      >
        <ColorWheel
          value={color ?? ""}
          disabled={disabled}
          onChange={(next) => onChange("lighting.ambient_color", next)}
        />
      </Control>
      )}

      {supportsColorTemp && (
      <Control
        label="Colour temperature"
        hint="Applied to the ambient lights that have a white channel. Low is warm."
        disabled={disabled}
        summary={kelvin === undefined ? "not set" : `${kelvin}K`}
        face={<KelvinSwatch value={kelvin} />}
      >
        <ValueSlider
          label="Colour temperature"
          value={kelvin}
          min={KELVIN_MIN}
          max={KELVIN_MAX}
          step={50}
          fallback={2200}
          unit="K"
          onCommit={(next) => onChange("lighting.ambient_kelvin", next)}
        />
      </Control>
      )}
    </div>
  );
}

function Control({
  label,
  hint,
  summary,
  face,
  disabled,
  children,
}: {
  label: string;
  hint: string;
  summary: string;
  face: React.ReactNode;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Popover>
      <PopoverTrigger
        disabled={disabled}
        aria-label={`${label} — ${summary}`}
        className={cn(
          "ring-border flex size-9 items-center justify-center rounded-full ring-1 transition-[box-shadow,opacity]",
          "hover:ring-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          disabled && "cursor-not-allowed opacity-50",
        )}
      >
        {face}
      </PopoverTrigger>
      <PopoverContent className="w-64">
        <PopoverTitle>{label}</PopoverTitle>
        <PopoverDescription className="mt-0.5 mb-3">{hint}</PopoverDescription>
        {children}
      </PopoverContent>
    </Popover>
  );
}

/**
 * A slider that writes when you let go.
 *
 * Every drag position is a real config write that lands on real lights,
 * so committing continuously would fill the undo history and spam the
 * broker with values nobody asked for.
 */
function ValueSlider({
  label,
  value,
  min,
  max,
  step,
  fallback,
  unit,
  onCommit,
}: {
  label: string;
  value?: number;
  min: number;
  max: number;
  step?: number;
  /** Where the thumb starts when nothing is configured yet. */
  fallback: number;
  unit: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(value ?? fallback);
  useEffect(() => setDraft(value ?? fallback), [value, fallback]);

  return (
    <Slider
      value={draft}
      min={min}
      max={max}
      step={step}
      onValueChange={(next) => setDraft(next)}
      onValueCommitted={(next) => onCommit(next)}
    >
      <div className="mb-1 flex items-baseline justify-between">
        <SliderValue className="text-muted-foreground">
          {() => `${draft}${unit}`}
        </SliderValue>
        {value === undefined && (
          <span className="text-muted-foreground/70 text-[11px]">not set</span>
        )}
      </div>
      <SliderControl>
        <SliderTrack>
          <SliderIndicator />
          {/* Base UI puts the range input inside the thumb, so the
              label belongs here rather than on the root. */}
          <SliderThumb aria-label={label} />
        </SliderTrack>
      </SliderControl>
    </Slider>
  );
}
