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
import { ColorWheel, kelvinToCss, parseHex } from "@/components/ColorField";
import { cn } from "@/lib/utils";

export interface AmbientControlsProps {
  brightness?: number;
  /** `#rrggbb`, or undefined when nothing is set. */
  color?: string;
  kelvin?: number;
  disabled?: boolean;
  onChange: (path: string, value: unknown) => void;
}

/** Sane ends for a lamp, not the 1000–10000 the config will accept. */
const KELVIN_MIN = 1800;
const KELVIN_MAX = 6500;

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
  brightness,
  color,
  kelvin,
  disabled,
  onChange,
}: AmbientControlsProps) {
  // Colour and colour temperature are two modes of one lamp, and the
  // config resolves that by preferring colour. Saying so here is better
  // than letting someone set a temperature that quietly does nothing.
  const colorWins = color !== undefined;

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Control
        label="Brightness"
        hint="How bright ambient lights sit, as a percentage."
        disabled={disabled}
        summary={brightness === undefined ? "not set" : `${brightness}%`}
        face={<BrightnessFace value={brightness} />}
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

      <Control
        label="Colour"
        hint="For RGB lights. Takes precedence over colour temperature."
        disabled={disabled}
        summary={color ?? "not set"}
        face={<ColorFace value={color} />}
      >
        <ColorWheel
          value={color ?? ""}
          disabled={disabled}
          onChange={(next) => onChange("lighting.ambient_color", next)}
        />
      </Control>

      <Control
        label="Colour temperature"
        hint={
          colorWins
            ? "Ignored while a colour is set — a light is in one mode or the other."
            : "For lights with a white channel. Low is warm."
        }
        disabled={disabled}
        muted={colorWins}
        summary={kelvin === undefined ? "not set" : `${kelvin}K`}
        face={<KelvinFace value={kelvin} />}
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
    </div>
  );
}

function Control({
  label,
  hint,
  summary,
  face,
  disabled,
  muted,
  children,
}: {
  label: string;
  hint: string;
  summary: string;
  face: React.ReactNode;
  disabled?: boolean;
  muted?: boolean;
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
          // Still reachable, still shows its value — just clearly not
          // the one in force.
          muted && "opacity-45",
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
      aria-label={label}
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
          <SliderThumb />
        </SliderTrack>
      </SliderControl>
    </Slider>
  );
}

/** A dial that fills to the level it holds. */
function BrightnessFace({ value }: { value?: number }) {
  const level = value ?? 0;
  return (
    <span
      aria-hidden
      className="border-border size-6 rounded-full border"
      style={{
        background: `conic-gradient(var(--foreground) ${level}%, var(--muted) ${level}%)`,
      }}
    />
  );
}

/** The chosen colour, or the wheel itself when there isn't one. */
function ColorFace({ value }: { value?: string }) {
  const rgb = value ? parseHex(value) : null;
  return (
    <span
      aria-hidden
      className="border-border size-6 rounded-full border"
      style={{
        background: rgb
          ? value
          : "conic-gradient(#ff0000, #ffff00, #00ff00, #00ffff, #0000ff, #ff00ff, #ff0000)",
      }}
    />
  );
}

/** The white it holds, over the warm-to-cool range it can hold. */
function KelvinFace({ value }: { value?: number }) {
  return (
    <span
      aria-hidden
      className="border-border size-6 rounded-full border"
      style={{
        background: value
          ? kelvinToCss(value)
          : `linear-gradient(to bottom, ${kelvinToCss(KELVIN_MIN)}, ${kelvinToCss(KELVIN_MAX)})`,
      }}
    />
  );
}
