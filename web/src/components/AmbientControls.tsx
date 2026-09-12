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
import { ColorWheel, parseHex } from "@/components/ColorField";
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
        face={<BrightnessFace />}
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
        face={<ColorFace value={color} />}
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

/**
 * A brightness glyph, not a gauge.
 *
 * An earlier version filled to the level, which nobody could read: the
 * unfilled part is dark on a dark card, so there was no container to
 * read the level against. The number lives inside, where there is room
 * for it.
 */
function BrightnessFace() {
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      className="size-5 fill-current"
      focusable="false"
    >
      <path d="M12,18V6A6,6 0 0,1 18,12A6,6 0 0,1 12,18M20,15.31L23.31,12L20,8.69V4H15.31L12,0.69L8.69,4H4V8.69L0.69,12L4,15.31V20H8.69L12,23.31L15.31,20H20V15.31Z" />
    </svg>
  );
}

/**
 * The chosen colour, or the wheel itself when there isn't one.
 *
 * The unset face is the same wheel the popover opens: hue around the
 * rim, white in the middle, so the button looks like the thing it
 * leads to.
 */
function ColorFace({ value }: { value?: string }) {
  const rgb = value ? parseHex(value) : null;
  return (
    <span
      aria-hidden
      className="border-border size-6 rounded-full border"
      style={
        rgb
          ? { background: value }
          : {
              backgroundImage: [
                "radial-gradient(circle closest-side, #ffffff, rgba(255,255,255,0) 78%)",
                "conic-gradient(from 90deg, #ff0000, #ff00ff, #0000ff, #00ffff, #00ff00, #ffff00, #ff0000)",
              ].join(", "),
            }
      }
    />
  );
}

/** Cool at one end, warm at the other, white through the middle. */
const KELVIN_COOL = [166, 209, 255] as const;
const KELVIN_WARM = [255, 160, 0] as const;

/**
 * The white it holds, over the warm-to-cool range it can hold.
 *
 * The swatch is taken from the same ramp the unset face shows, rather
 * than from the blackbody curve the chart uses. A physically accurate
 * 2200 K is a muddy brown at thumbnail size; this reads as a warm lamp,
 * which is what the setting means.
 */
function KelvinFace({ value }: { value?: number }) {
  return (
    <span
      aria-hidden
      className="border-border size-6 rounded-full border"
      style={{
        background: value
          ? kelvinSwatch(value)
          : `linear-gradient(90deg, rgb(${KELVIN_WARM.join(", ")}) 0%, rgb(255, 255, 255) 50%, rgb(${KELVIN_COOL.join(", ")}) 100%)`,
      }}
    />
  );
}

function kelvinSwatch(kelvin: number): string {
  const span = KELVIN_MAX - KELVIN_MIN;
  const t = Math.min(Math.max((kelvin - KELVIN_MIN) / span, 0), 1);
  const white = [255, 255, 255] as const;
  const [from, to, mix] =
    t < 0.5 ? [KELVIN_WARM, white, t * 2] : [white, KELVIN_COOL, (t - 0.5) * 2];
  const channel = (i: number) => Math.round(from[i] + (to[i] - from[i]) * mix);
  return `rgb(${channel(0)}, ${channel(1)}, ${channel(2)})`;
}
